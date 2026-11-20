//! Applying account or unsigned-developer sessions to GUI Play.
use super::*;

impl Launcher {
    pub(super) fn handle_game_session(
        &mut self,
        session: std::result::Result<Option<account::GameSession>, account::NoOwner>,
    ) {
        self.load_auth_statuses(false);
        if let (Some(tag), Some(intent)) = (self.starting_client.take(), self.starting_play.take())
        {
            let Some(owner) = intent.owner else {
                if let Ok(Some(session)) = session {
                    account::discard_launch_session(session.file);
                }
                self.client_error = Some(
                    tr("This profile no longer exists. Choose another profile to play.").into(),
                );
                return;
            };
            let pass = match session {
                Ok(pass) => pass,
                Err(refused) => {
                    // A keyring that didn't answer isn't a missing owner (#12179).
                    if refused.keyring.is_none() {
                        self.owner = Some(false);
                        self.owner_screen_open = true;
                    }
                    self.client_error = Some(refused.in_window());
                    self.starting_pack = None;
                    self.joining = None;
                    self.quick_play = None;
                    return;
                }
            };
            let session_file = pass.as_ref().and_then(|session| session.file.clone());
            let mut cmd = match backend::managed_client_command(&tag, &owner) {
                Ok(command) => command,
                Err(error) => {
                    self.client_error = Some(error);
                    account::discard_launch_session(session_file);
                    return;
                }
            };
            if let Some(session) = pass {
                self.owner = Some(true);
                // Still play, offline, and say why.
                if let Some(e) = session.apply(&mut cmd) {
                    self.client_error = Some(trf("Playing offline: {reason}", &[("reason", &e)]));
                } else if let Some(e) = account::take_unsaved() {
                    self.client_error = Some(e);
                }
            } else {
                if let Err(error) = sunrise_launcher::developer_offline::require_capability(&tag) {
                    self.client_error = Some(error);
                    self.starting_pack = None;
                    self.joining = None;
                    self.quick_play = None;
                    return;
                }
                sunrise_launcher::developer_offline::mark(&mut cmd);
                self.client_error = Some(tr(sunrise_launcher::developer_offline::NOTICE).into());
            }
            // A pack's Play button plays that pack; Play plays the selected instance.
            let (profile, run) = match self.starting_pack.take() {
                Some(work) => {
                    let id = instances::list()
                        .into_iter()
                        .find(|i| i.work == work)
                        .map(|i| i.id);
                    let applied = sunrise_launcher::browser::sunrise::pack::apply(&mut cmd, &work);
                    (applied, id)
                }
                None => {
                    let Some(instance) = instances::list()
                        .into_iter()
                        .find(|i| i.id == intent.instance && i.home == owner.home())
                    else {
                        self.client_error = Some(
                            tr("This profile no longer exists. Choose another profile to play.")
                                .into(),
                        );
                        self.joining = None;
                        self.quick_play = None;
                        account::discard_launch_session(session_file);
                        return;
                    };
                    (instances::apply(&mut cmd, &instance), Some(instance.id))
                }
            };
            if let Err(e) = profile {
                self.client_error = Some(e);
                account::discard_launch_session(session_file);
                return;
            }
            if let Some(code) = self.joining.take() {
                if let Err(e) = backend::join_friend(&mut cmd, &code) {
                    self.client_error = Some(e);
                }
            }
            if let Some((flag, value)) = self.quick_play.take() {
                cmd.arg(flag).arg(value);
            }
            backend::append_game_args(&mut cmd);
            // Last, around every argument: the instance's pre-launch command,
            // wrapper and variables.
            let instance = run
                .as_ref()
                .and_then(|id| instances::list().into_iter().find(|i| &i.id == id));
            self.client_vars = instance.as_ref().map(|i| i.vars()).unwrap_or_default();
            cmd.envs(self.client_vars.iter().map(|(k, v)| (k, v)));
            let launch = instance.map(|i| i.launch).unwrap_or_default();
            self.starting_client = Some(tag.clone());
            jar_launch::client(
                cmd,
                tag,
                launch,
                run,
                (intent.account, owner),
                session_file,
                self.tx.clone(),
            );
        } else if let Ok(Some(session)) = session {
            account::discard_launch_session(session.file);
        }
    }
}
