//! Serialize native grant changes, general settings writes and native work admission. Only the
//! two permission fields change merge policy; other Settings fields retain their existing rules.
use super::{data_dir, write_private, Settings, SourceModGrant};
use crate::i18n::tr;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

const SETTINGS: &str = "settings.json";
const ALLOWED: &str = "native_mods_allowed_at";
const REVISION: &str = "native_consent_revision";
const SOURCE_GRANTS: &str = "source_mod_grants";

#[derive(Deserialize, Default)]
#[serde(default)]
struct Permission {
    native_mods_allowed_at: Option<u64>,
    native_consent_revision: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct NativeGrant(u64);

// Owned file lock, released when the caller drops it. Never hold it through a build or dialog.
pub(crate) struct PermissionFence {
    _gate: fs::File,
    path: PathBuf,
}

impl PermissionFence {
    fn open(data: &Path) -> Result<Self, String> {
        let path = data.join(SETTINGS);
        let result = (|| {
            fs::create_dir_all(data)?;
            let gate = fs::File::options()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(data.join(".settings.json.lock"))?;
            gate.lock()?;
            Ok::<_, io::Error>(gate)
        })();
        let gate = result.map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Self { _gate: gate, path })
    }

    fn read<T: Default + serde::de::DeserializeOwned>(&self) -> Result<T, String> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(T::default()),
            Err(e) => return Err(format!("{}: {e}", self.path.display())),
        };
        serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", self.path.display()))
    }

    fn grant(&self) -> Result<Option<NativeGrant>, String> {
        let live: Permission = self.read()?;
        Ok(live
            .native_mods_allowed_at
            .map(|_| NativeGrant(live.native_consent_revision)))
    }

    fn write(&self, fields: &Map<String, Value>) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(fields).map_err(|e| e.to_string())?;
        let dir = self.path.parent().expect("settings directory");
        let temp = tempfile::Builder::new()
            .prefix(".settings-")
            .tempfile_in(dir)
            .map_err(|e| format!("{}: {e}", self.path.display()))?;
        write_private(temp.path(), &bytes)
            .and_then(|()| temp.as_file().sync_all())
            .map_err(|e| format!("{}: {e}", self.path.display()))?;
        temp.persist(&self.path)
            .map_err(|e| format!("{}: {e}", self.path.display()))?;
        #[cfg(unix)]
        fs::File::open(dir)
            .and_then(|parent| parent.sync_all())
            .map_err(|e| format!("{}: {e}", self.path.display()))?;
        Ok(())
    }
}

impl NativeGrant {
    pub(crate) fn read() -> Result<Option<Self>, String> {
        PermissionFence::open(&data_dir())?.grant()
    }

    pub(crate) fn hold(self) -> Result<PermissionFence, String> {
        self.hold_in(&data_dir())
    }

    fn hold_in(self, data: &Path) -> Result<PermissionFence, String> {
        let file = PermissionFence::open(data)?;
        if file.grant()? != Some(self) {
            return Err(tr("Native mod permission changed. Start again.").into());
        }
        Ok(file)
    }
}

impl Settings {
    /// An ordinary Settings save can neither grant nor revoke native-code permission.
    pub fn save(&self) -> Result<(), String> {
        self.save_to(&data_dir())
    }

    fn save_to(&self, data: &Path) -> Result<(), String> {
        let file = PermissionFence::open(data)?;
        let live: Map<String, Value> = file.read()?;
        let Value::Object(mut now) = serde_json::to_value(self).map_err(|e| e.to_string())? else {
            unreachable!("Settings serializes as an object");
        };
        for key in [ALLOWED, REVISION, SOURCE_GRANTS] {
            if let Some(value) = live.get(key) {
                now.insert(key.into(), value.clone());
            } else {
                now.remove(key);
            }
        }
        file.write(&now)
    }

    /// Persist the exact source grants and, on first use, the introductory native grant
    /// in the same fenced settings write. Do not replay another launcher's permissions.
    pub fn allow_source_mods(&mut self, approved: Vec<SourceModGrant>) -> Result<(), String> {
        self.allow_source_mods_in(&data_dir(), approved)
    }

    fn allow_source_mods_in(
        &mut self,
        data: &Path,
        approved: Vec<SourceModGrant>,
    ) -> Result<(), String> {
        if approved.is_empty() {
            return Err(tr("The source could not be verified.").into());
        }
        let file = PermissionFence::open(data)?;
        let mut live: Map<String, Value> = file.read()?;
        let prior: Permission = serde_json::from_value(Value::Object(live.clone()))
            .map_err(|e| format!("{SETTINGS}: {e}"))?;
        let mut grants: Vec<SourceModGrant> = live
            .remove(SOURCE_GRANTS)
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        for grant in approved {
            if !grants.contains(&grant) {
                grants.push(grant);
            }
        }
        let time = if let Some(current) = prior.native_mods_allowed_at {
            current
        } else {
            let time = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|error| error.to_string())?
                .as_secs();
            let revision = prior
                .native_consent_revision
                .checked_add(1)
                .ok_or_else(|| tr("Native mod permission could not be saved.").to_owned())?;
            live.insert(ALLOWED.into(), time.into());
            live.insert(REVISION.into(), revision.into());
            time
        };
        live.insert(
            SOURCE_GRANTS.into(),
            serde_json::to_value(&grants).map_err(|error| error.to_string())?,
        );
        file.write(&live)?;
        self.native_mods_allowed_at = Some(time);
        self.source_mod_grants = grants;
        Ok(())
    }

    pub fn revoke_native_mods(&mut self) -> Result<(), String> {
        self.set_native_permission(&data_dir(), None)
    }

    pub(super) fn set_native_permission(
        &mut self,
        data: &Path,
        allowed: Option<u64>,
    ) -> Result<(), String> {
        let file = PermissionFence::open(data)?;
        let mut live: Map<String, Value> = file.read()?;
        let prior: Permission = serde_json::from_value(Value::Object(live.clone()))
            .map_err(|e| format!("{SETTINGS}: {e}"))?;
        let revision = prior
            .native_consent_revision
            .checked_add(1)
            .ok_or_else(|| tr("Native mod permission could not be saved.").to_owned())?;
        if let Some(time) = allowed {
            live.insert(ALLOWED.into(), time.into());
        } else {
            live.remove(ALLOWED);
        }
        live.insert(REVISION.into(), revision.into());
        file.write(&live)?;
        self.native_mods_allowed_at = allowed;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_grants_use_repo_id_owner_and_mod_and_survive_stale_settings_saves() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path();
        let stale = Settings::default();
        let mut editor = Settings::default();
        let grant = |repo_id, owner: &str, id: &str| SourceModGrant {
            repo_id,
            owner: owner.into(),
            id: id.into(),
        };
        let first = grant(1, "river", "trail");
        editor
            .allow_source_mods_in(data, vec![first.clone()])
            .unwrap();
        let time = editor
            .native_mods_allowed_at
            .expect("first source grant also grants native permission");
        let initial = PermissionFence::open(data).unwrap().grant().unwrap();
        for different in [
            grant(2, "river", "trail"),
            grant(1, "grove", "trail"),
            grant(1, "river", "camp"),
        ] {
            assert!(!editor.source_mod_grants.contains(&different));
        }
        let second = grant(1, "river", "camp");
        editor
            .allow_source_mods_in(data, vec![first.clone(), second.clone()])
            .unwrap();
        stale.save_to(data).unwrap();
        let loaded = Settings::load_from(data);
        assert_eq!(loaded.source_mod_grants, [first, second]);
        assert_eq!(loaded.native_mods_allowed_at, Some(time));
        assert_eq!(
            PermissionFence::open(data).unwrap().grant().unwrap(),
            initial
        );
    }

    #[test]
    fn old_settings_writers_cannot_replay_permission_and_new_grants_do_not_authorize_old_work() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path();
        let mut editor = Settings::default();
        editor.set_native_permission(data, Some(123)).unwrap();
        let old = PermissionFence::open(data)
            .unwrap()
            .grant()
            .unwrap()
            .unwrap();
        drop(old.hold_in(data).unwrap());
        let mut stale = Settings::load_from(data);
        assert_eq!(stale.native_mods_allowed_at, Some(123));
        editor.set_native_permission(data, None).unwrap();
        stale.sidebar_expanded = true;
        stale.save_to(data).unwrap();
        assert!(Settings::load_from(data).sidebar_expanded);
        assert_eq!(Settings::load_from(data).native_mods_allowed_at, None);
        assert!(old.hold_in(data).is_err());
        editor.set_native_permission(data, Some(123)).unwrap();
        assert!(old.hold_in(data).is_err());
        let new = PermissionFence::open(data)
            .unwrap()
            .grant()
            .unwrap()
            .unwrap();
        drop(new.hold_in(data).unwrap());
    }
}
