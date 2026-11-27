//! Instances in the window (`sunrise_launcher::instances`): the sidebar's list, by group, and the
//! dialog that makes one (empty, or a copy of the selected one) or changes its name, group and
//! icon.
use super::*;
use gpui::RenderImage;
use servers_page::Field;
use std::collections::HashMap;
use std::sync::Arc;
use sunrise_launcher::instances::{self, Icon, Instance, Kind as InstanceKind};

mod unsaved_edits;

/// The dialog: a new instance (`editing` None) or one being changed. Its name and group are the
/// text fields `Field::InstanceName` and `Field::InstanceGroup`.
pub(super) struct InstanceDialog {
    pub(super) editing: Option<String>,
    /// The selected source and build shown when copying its worlds, options and packs.
    pub(super) copy: Option<instances::CopyPreview>,
    pub(super) icon: Option<Icon>,
    /// A picture chosen since it opened (None in it: the instance's is removed on Save).
    pub(super) picture: Option<Option<instance_icons::Chosen>>,
    pub(super) error: Option<String>,
    pub(super) busy: bool,
    /// The Sunrise version it keeps (`Instance::version`); None follows the launcher's.
    pub(super) pin: Option<String>,
    /// The builds "latest" follows (`Instance::channel`).
    pub(super) channel: instances::Channel,
    /// The sheet was closed with edits not saved: asking whether to drop them.
    pub(super) discarding: bool,
    /// The dropdown open under one of its fields.
    pub(super) menu: Option<DialogMenu>,
    /// The version list under Game build is showing.
    pub(super) pinning: bool,
}

/// The dropdowns of the profile dialog.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DialogMenu {
    Build,
    Start,
    Group,
}

/// The instances and their icons as images, kept between frames.
#[derive(Default)]
pub(super) struct Instances {
    pub(super) list: Vec<Instance>,
    pub(super) dialog: Option<InstanceDialog>,
    /// Textures, pictures and marks as images, by name and size in device pixels.
    pub(super) icons: HashMap<(String, u32), Option<Arc<RenderImage>>>,
}

impl Instances {
    pub(super) fn selected(&self, id: Option<&str>) -> Option<&Instance> {
        let id = id.unwrap_or(instances::DEFAULT);
        // A saved id that is gone falls back to the default profile, wherever it now sorts.
        self.list
            .iter()
            .find(|i| i.id == id)
            .or_else(|| self.list.iter().find(|i| i.id == instances::DEFAULT))
            .or(self.list.first())
    }
}

impl Launcher {
    pub(super) fn reload_instances(&mut self) {
        self.refresh_instances_list();
        // A saved selection an earlier launcher made (a ported pack) can't be used: it is
        // cleared once, with a line saying so, and Sunrise shows.
        if let Some(id) =
            instances::retired_selection(&self.instances.list, self.settings.instance.as_deref())
        {
            self.settings.instance = None;
            self.save_settings();
            if !matches!(
                &self.ui.sheets.profile_action,
                Some(crate::profile_delete::Action::Interrupted(notices))
                    if notices.iter().any(|notice| notice.id() == id)
            ) {
                crate::toast::say(instances::retired_selection_message(&id));
            }
        }
        let _ = self.skins_offline();
    }

    pub(super) fn refresh_instances_list(&mut self) {
        self.instances.list = instances::list();
        // A picture may have come or gone with the change.
        self.instances
            .icons
            .retain(|(key, _), _| !key.starts_with("picture:"));
        self.refresh_open_storage_builds();
    }

    /// The instance Play starts.
    pub(super) fn selected_instance(&self) -> Option<&Instance> {
        self.instances.selected(self.settings.instance.as_deref())
    }

    /// The profile's Play target, including a build Play will have to install first.
    pub(crate) fn profile_target(&self, instance: &Instance) -> Option<String> {
        if instance.kind != instances::Kind::Pack {
            match sunrise_launcher::managed_play::display_active(&instance.home) {
                Ok(Some(active)) => return Some(active.base().to_owned()),
                // Our install or switch may hold this profile's admission. Keep the base it
                // captured visible, rather than treating the profile as uninstalled: Discover and
                // the build chooser would otherwise read "No game build selected" until it ends.
                Err(_) => {
                    return self
                        .install_base(instance)
                        .or_else(|| self.switch_base(&instance.home))
                        .map(str::to_owned)
                }
                Ok(None) => (),
            }
        }
        self.target_version(instance.version.as_ref(), instance.channel)
            .cloned()
    }

    /// The installed executable Play starts; Home's picker separately shows its base.
    pub(super) fn version_of(&self, instance: &Instance) -> Option<&String> {
        let target = if instance.kind != instances::Kind::Pack {
            match sunrise_launcher::managed_play::display_active(&instance.home).ok()? {
                Some(active) => {
                    return self
                        .installed
                        .iter()
                        .find(|tag| tag.as_str() == active.executable())
                }
                None => self.target_version(instance.version.as_ref(), instance.channel)?,
            }
        } else {
            self.target_version(instance.version.as_ref(), instance.channel)?
        };
        self.installed.iter().find(|tag| *tag == target)
    }

    /// The Minecraft version `instance`'s own build plays: what its mods and packs are for.
    pub(super) fn game_of(&self, instance: &Instance) -> String {
        backend::played_version_id_by(self.version_of(instance).map(String::as_str))
    }

    /// The Minecraft version the Mods page installs and updates for: the selected instance's build, else the launcher's.
    pub(super) fn mods_game(&self) -> String {
        self.selected_instance()
            .map_or_else(sunrise_launcher::browser::game, |i| self.game_of(i))
    }

    pub(super) fn select_instance(&mut self, instance: &Instance) {
        self.settings.instance =
            (instance.kind != InstanceKind::Default).then(|| instance.id.clone());
        self.save_settings();
        let _ = self.skins_offline();
    }

    /// Closes the dialog; a sheet holding edits not saved asks first. A dialog that is saving
    /// stays.
    pub(super) fn close_instance_dialog(&mut self) {
        // A new profile has nothing saved to compare with: what was typed or picked in it is
        // the edit.
        let typed = !self.servers.value(Field::InstanceName).trim().is_empty()
            || !self.servers.value(Field::InstanceGroup).trim().is_empty();
        let changed = self.sheet_changed()
            || self.instances.dialog.as_ref().is_some_and(|d| {
                d.editing.is_none() && (typed || d.icon.is_some() || d.picture.is_some())
            });
        if let Some(d) = self.instances.dialog.as_mut().filter(|d| !d.busy) {
            if changed {
                d.discarding = true;
            } else {
                self.instances.dialog = None;
            }
        }
    }

    /// Escape closes the dialog, or answers the question about the edits with "Keep editing".
    /// Whether it did anything.
    pub(super) fn instance_escape(&mut self) -> bool {
        match self.instances.dialog.as_mut() {
            Some(d) if d.busy => false,
            Some(d) if d.discarding => {
                d.discarding = false;
                true
            }
            Some(_) => {
                self.close_instance_dialog();
                true
            }
            None => false,
        }
    }

    /// Why `profile` can't be copied now: the game is running from its folder.
    pub(super) fn copy_lock_for(&self, profile: &Instance) -> Option<&'static str> {
        (self.client_home.as_ref() == Some(&profile.home) && self.client.is_some())
            .then(|| tr("Stop the game before copying the profile it is using."))
    }

    /// Creates or changes the instance as the dialog says. A copy can be gigabytes of worlds, so
    /// the work runs in the background.
    pub(super) fn save_instance(&mut self, cx: &mut Context<Self>) {
        // The rename-a-group box shares the group field; its Enter lands here.
        if self.commit_group_name(cx) {
            return;
        }
        let edited_id = self
            .instances
            .dialog
            .as_ref()
            .filter(|d| !d.busy)
            .and_then(|d| d.editing.clone());
        if edited_id.is_some() {
            // Nothing to save, as the footer's Save says.
            if !self.sheet_changed() {
                return;
            }
        }
        let launch = match edited_id
            .as_deref()
            .and_then(|id| self.parsed_launch(id))
            .transpose()
        {
            Ok(launch) => launch,
            Err(e) => {
                if let Some(d) = &mut self.instances.dialog {
                    d.error = Some(e);
                }
                return;
            }
        };
        let source = self
            .instances
            .dialog
            .as_ref()
            .and_then(|d| d.copy.as_ref())
            .map(|copy| copy.source().clone());
        if let Some(why) = source.as_ref().and_then(|s| self.copy_lock_for(s)) {
            if let Some(d) = self.instances.dialog.as_mut() {
                d.error = Some(why.into());
                return;
            }
        }
        let name = self.servers.value(Field::InstanceName).trim().to_string();
        // An empty field is no group, so the sidebar has no blank heading for it.
        let group = Some(self.servers.value(Field::InstanceGroup).trim().to_string())
            .filter(|g| !g.is_empty());
        let Some(d) = self.instances.dialog.as_mut().filter(|d| !d.busy) else {
            return;
        };
        let icon = d.icon.clone();
        let (pin, channel) = (d.pin.clone(), d.channel);
        let picture = d
            .picture
            .as_ref()
            .map(|p| p.as_ref().map(|c| c.png.clone()));
        let editing = d.editing.clone();
        let edited = editing.is_some();
        let copy = d.copy.clone();
        // The picture choice this save keeps: once kept, it is no longer an unsaved edit.
        let sent_picture = picture.clone();
        let Some(d) = self.instances.dialog.as_mut() else {
            return;
        };
        d.busy = true;
        d.error = None;
        let task = cx.background_executor().spawn(async move {
            let called = name.clone();
            let choose = |m: &mut instances::Meta| {
                m.name = Some(name);
                m.group = group;
                m.icon = icon;
                if let Some(launch) = launch {
                    m.launch = launch;
                }
                m.version = pin;
                m.channel = Some(channel);
            };
            let Some(id) = editing else {
                // Carry the selected folder's identity into the worker rather than looking it
                // up again by slug after another launcher might have replaced it.
                return instances::create_with(
                    &called,
                    copy.as_ref(),
                    picture.as_ref().map(Option::as_deref),
                    choose,
                );
            };
            // Finish fallible picture work before committing the edited metadata and commands.
            if let Some(png) = picture {
                let instance = instances::find(&id)?;
                instances::set_picture(
                    &instance.home,
                    png.as_deref(),
                    &sunrise_launcher::desktop_seams::to_trash,
                )?;
            }
            let instance = instances::edit(&id, choose)?;
            Ok(instance)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(instance) => {
                        let name = [("name", instance.name.as_str())];
                        crate::toast::say(if edited {
                            trf("Saved {name}", &name)
                        } else {
                            trf("Created {name}", &name)
                        });
                        this.select_instance(&instance);
                        this.reload_instances();
                        if let Some(d) = &mut this.instances.dialog {
                            let now = d.picture.as_ref().map(|p| p.as_ref().map(|c| &c.png));
                            if now == sent_picture.as_ref().map(Option::as_ref) {
                                d.picture = None;
                            }
                        }
                        // Edits typed while it saved are still open: the sheet stays for them.
                        let typed_meanwhile = edited && this.sheet_changed();
                        match (typed_meanwhile, &mut this.instances.dialog) {
                            (true, Some(d)) => d.busy = false,
                            _ => this.instances.dialog = None,
                        }
                        // Nothing downloads until the player presses Install (or Play): a new
                        // profile only shows what it would install.
                        this.refresh_installed();
                        if matches!(instance.kind, InstanceKind::Default | InstanceKind::Own) {
                            this.navigate(crate::frame::Page::Home);
                        }
                    }
                    Err(e) => {
                        if let Some(d) = &mut this.instances.dialog {
                            d.busy = false;
                            d.error = Some(e);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
