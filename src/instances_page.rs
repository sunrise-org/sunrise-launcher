//! Instances in the window (`sunrise_launcher::instances`): the sidebar's list, by group, and the
//! dialog that makes one (empty, or a copy of the selected one) or changes its name, group and
//! icon.
use super::*;
use gpui::RenderImage;
use servers_page::Field;
use std::collections::HashMap;
use std::sync::Arc;
use sunrise_launcher::instances::{self, Icon, Instance, Kind as InstanceKind};

/// The dialog: a new instance (`editing` None) or one being changed. Its name and group are the
/// text fields `Field::InstanceName` and `Field::InstanceGroup`.
pub(super) struct InstanceDialog {
    pub(super) editing: Option<String>,
    /// A new instance copies the selected one's worlds, options and packs.
    pub(super) copy: bool,
    pub(super) icon: Option<Icon>,
    /// A picture chosen since it opened (None in it: the instance's is removed on Save).
    pub(super) picture: Option<Option<instance_icons::Chosen>>,
    pub(super) picture_focus: FocusHandle,
    pub(super) error: Option<String>,
    pub(super) busy: bool,
    /// The Sunrise version it keeps (`Instance::version`); None follows the launcher's.
    pub(super) pin: Option<String>,
    /// The builds "latest" follows (`Instance::channel`).
    pub(super) channel: instances::Channel,
    /// The sheet was closed with edits not saved: asking whether to drop them.
    pub(super) discarding: bool,
}

/// The instances and their icons as images, kept between frames.
#[derive(Default)]
pub(super) struct Instances {
    pub(super) list: Vec<Instance>,
    pub(super) dialog: Option<InstanceDialog>,
    /// Textures, pictures and marks as images, by name and size in device pixels.
    pub(super) icons: HashMap<(String, u32), Option<Arc<RenderImage>>>,
}

impl Launcher {
    pub(super) fn reload_instances(&mut self) {
        self.instances.list = instances::list();
        // A saved selection an earlier launcher made (a ported pack) can't be used: it is
        // cleared once, with a line saying so, and Sunrise shows.
        if let Some(id) = instances::retired_selection() {
            self.settings.instance = None;
            self.save_settings();
            crate::glass_toast::say(instances::retired_selection_message(&id));
        }
        // A picture may have come or gone with the change.
        self.instances
            .icons
            .retain(|(key, _), _| !key.starts_with("picture:"));
    }

    /// The instance Play starts.
    pub(super) fn selected_instance(&self) -> Option<&Instance> {
        let id = self
            .settings
            .instance
            .as_deref()
            .unwrap_or(instances::DEFAULT);
        let list = &self.instances.list;
        list.iter().find(|i| i.id == id).or(list.first())
    }

    /// The profile's Play target, including a build Play will have to install first.
    pub(super) fn profile_target(&self, instance: &Instance) -> Option<String> {
        if instance.kind != instances::Kind::Pack {
            match sunrise_launcher::managed_play::display_active(&instance.home) {
                Ok(Some(active)) => return Some(active.base().to_owned()),
                Err(_) => return None, // A claimed or unsettled profile must not display a false pin.
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
        // A native build's warning names the mods of the instance it was opened for.
        self.pending_consent = None;
        self.settings.instance =
            (instance.kind != InstanceKind::Default).then(|| instance.id.clone());
        self.save_settings();
    }

    /// Closes the dialog; a sheet holding edits not saved asks first. A dialog that is saving
    /// stays.
    pub(super) fn close_instance_dialog(&mut self) {
        let changed = self.sheet_changed();
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

    pub(super) fn profile_copy_lock(&self) -> Option<&'static str> {
        let selected = self.selected_instance()?;
        (self.client_home.as_ref() == Some(&selected.home) && self.client.is_some())
            .then(|| tr("Stop the game before copying the profile it is using."))
    }

    /// Creates or changes the instance as the dialog says. A copy can be gigabytes of worlds, so
    /// the work runs in the background.
    pub(super) fn save_instance(&mut self, cx: &mut Context<Self>) {
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
        if let Some(why) = self.profile_copy_lock() {
            if let Some(d) = self.instances.dialog.as_mut().filter(|d| d.copy) {
                d.error = Some(why.into());
                return;
            }
        }
        let selected = self.selected_instance().map(|i| i.id.clone());
        let name = self.servers.value(Field::InstanceName).trim().to_string();
        let group = Some(self.servers.value(Field::InstanceGroup).to_string());
        // A Sunrise version picked but not installed downloads once the instance is saved.
        let download = self
            .draft_sunrise()
            .and_then(|tag| self.release_to_install(&tag));
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
        let copy = selected.filter(|_| d.copy);
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
                // A new profile is made whole or not at all: a failure leaves none to retry over.
                return instances::create_with(
                    &called,
                    copy.as_deref(),
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
                        crate::glass_toast::say(if edited {
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
                        // Play shows the download's progress; Versions shows it too.
                        if let Some(release) = download {
                            this.install_for_instance(release);
                        }
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
