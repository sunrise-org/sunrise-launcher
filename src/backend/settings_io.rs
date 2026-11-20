//! Merge a Settings editor's intended changes without replaying its view of native consent.
use super::{data_dir, Settings};
use crate::local_json::{merge_changes, LockedJson};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::{path::Path, sync::Mutex};

const FILE: &str = "settings.json";
const ALLOWED: &str = "native_mods_allowed_at";
const REVISION: &str = "native_consent_revision";

/// An editor remembers the values it read, not the raw keys it didn't understand.
#[derive(Default)]
#[doc(hidden)]
pub struct SettingsBaseline(Mutex<Option<Map<String, Value>>>);

#[derive(Default, Deserialize)]
#[serde(default)]
struct NativePermission {
    native_mods_allowed_at: Option<u64>,
    native_consent_revision: u64,
}

/// The grant under which native work was admitted, not the time displayed in Settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NativeGrant(u64);

impl NativeGrant {
    pub(crate) fn hold(self) -> Result<LockedJson, String> {
        self.hold_in(&data_dir())
    }

    pub(crate) fn hold_in(self, dir: &Path) -> Result<LockedJson, String> {
        let file = LockedJson::open(dir, FILE)?;
        if NativePermission::grant(&file)? == Some(self) {
            Ok(file)
        } else {
            Err(crate::i18n::tr("Native mod permission changed. Start again.").into())
        }
    }
}

impl NativePermission {
    fn grant(file: &LockedJson) -> Result<Option<NativeGrant>, String> {
        let permission: Self = file.read()?;
        Ok(permission
            .native_mods_allowed_at
            .map(|_| NativeGrant(permission.native_consent_revision)))
    }
}

impl Settings {
    pub(crate) fn native_grant() -> Result<Option<NativeGrant>, String> {
        Self::native_grant_from(&data_dir())
    }

    pub(crate) fn native_grant_from(data: &Path) -> Result<Option<NativeGrant>, String> {
        NativePermission::grant(&LockedJson::open(data, FILE)?)
    }

    pub(super) fn record_baseline(&mut self) {
        *self
            .settings_baseline
            .0
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(setting_fields(self).expect("Settings serialize as an object"));
    }

    pub fn save(&self) -> Result<(), String> {
        self.save_to(&data_dir())
    }

    fn save_to(&self, data: &Path) -> Result<(), String> {
        let now = setting_fields(self)?;
        let mut baseline = self
            .settings_baseline
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let file = LockedJson::open(data, FILE)?;
        let mut live: Map<String, Value> = file.read()?;
        if let Some(before) = baseline.as_ref() {
            merge_changes(before, &now, &mut live);
        } else {
            merge_changes(&setting_fields(&Self::default())?, &now, &mut live);
        }
        file.write(&live)?;
        // Record what this editor just saved; other writers' edits won't become ours on the
        // next save even if the window hasn't read them yet.
        *baseline = Some(now);
        Ok(())
    }

    /// Remember a new explicit approval; finding an installed compiler never grants it.
    pub fn allow_native_mods(&mut self) -> Result<(), String> {
        self.set_native_permission(
            &data_dir(),
            Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|e| e.to_string())?
                    .as_secs(),
            ),
        )
    }

    pub fn revoke_native_mods(&mut self) -> Result<(), String> {
        self.set_native_permission(&data_dir(), None)
    }

    fn set_native_permission(&mut self, dir: &Path, allowed: Option<u64>) -> Result<(), String> {
        let file = LockedJson::open(dir, FILE)?;
        let mut live: Map<String, Value> = file.read()?;
        let prior: NativePermission = serde_json::from_value(Value::Object(live.clone()))
            .map_err(|e| format!("{FILE}: {e}"))?;
        let revision = prior
            .native_consent_revision
            .checked_add(1)
            .ok_or_else(|| {
                crate::i18n::tr("Native mod permission could not be saved.").to_string()
            })?;
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

fn setting_fields(settings: &Settings) -> Result<Map<String, Value>, String> {
    match serde_json::to_value(settings).map_err(|e| e.to_string())? {
        Value::Object(mut fields) => {
            fields.remove(ALLOWED);
            Ok(fields)
        }
        _ => unreachable!("Settings serialize as an object"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_settings_save_changes_only_the_edit_and_cannot_regrant_permission() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let mut owner = Settings {
            cache_limit_gib: Some(50),
            ..Settings::default()
        };
        owner.save_to(dir).unwrap();
        owner.set_native_permission(dir, Some(123)).unwrap();
        let mut stale = Settings::load_from(dir);
        assert_eq!(stale.native_mods_allowed_at, Some(123));
        owner.set_native_permission(dir, None).unwrap();
        owner.game_args = "--local".into();
        owner.save_to(dir).unwrap();
        stale.sidebar_expanded = true;
        stale.save_to(dir).unwrap();
        let after = Settings::load_from(dir);
        assert_eq!(after.native_mods_allowed_at, None);
        assert_eq!(after.cache_limit_gib, Some(50));
        assert_eq!(after.game_args, "--local");
        assert!(after.sidebar_expanded);
        let consent = LockedJson::open(dir, FILE)
            .unwrap()
            .read::<NativePermission>()
            .unwrap();
        assert_eq!(consent.native_consent_revision, 2);
    }

    #[test]
    fn a_new_grant_never_authorizes_work_admitted_under_a_revoked_revision() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let mut editor = Settings::load_from(dir);
        editor.set_native_permission(dir, Some(123)).unwrap();
        let old = NativePermission::grant(&LockedJson::open(dir, FILE).unwrap())
            .unwrap()
            .unwrap();
        drop(old.hold_in(dir).unwrap());
        editor.set_native_permission(dir, None).unwrap();
        assert!(old.hold_in(dir).is_err());
        editor.set_native_permission(dir, Some(123)).unwrap();
        assert!(old.hold_in(dir).is_err());
        let new = NativePermission::grant(&LockedJson::open(dir, FILE).unwrap())
            .unwrap()
            .unwrap();
        drop(new.hold_in(dir).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn replacing_settings_keeps_credentials_private_even_over_an_old_looser_file() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let path = dir.join(FILE);
        std::fs::write(&path, b"{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let mut editor = Settings::load_from(dir);
        editor.proxy = Some("http://u:p@h:1".into());
        editor.save_to(dir).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(Settings::load_from(dir).proxy, editor.proxy);
    }

    #[test]
    fn independent_edits_to_a_setting_map_do_not_erase_one_another() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let mut a = Settings::load_from(dir);
        let mut b = Settings::load_from(dir);
        a.theme_accents
            .insert(crate::themes::DAWN_THEME.id.into(), a.accent);
        b.theme_accents.insert("twilight".into(), b.accent);
        a.save_to(dir).unwrap();
        b.save_to(dir).unwrap();
        assert_eq!(Settings::load_from(dir).theme_accents.len(), 2);
        a.theme_accents.clear();
        a.save_to(dir).unwrap();
        assert_eq!(
            Settings::load_from(dir)
                .theme_accents
                .into_keys()
                .collect::<Vec<_>>(),
            ["twilight"]
        );
    }

    #[test]
    fn malformed_live_settings_are_never_overwritten_by_a_stale_editor() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        for bytes in [b"{".as_slice(), b"[]".as_slice(), b"null".as_slice()] {
            std::fs::write(&path, bytes).unwrap();
            let mut editor = Settings {
                sidebar_expanded: true,
                ..Settings::default()
            };
            assert!(editor.save_to(dir.path()).is_err());
            assert!(editor.set_native_permission(dir.path(), None).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }
}
