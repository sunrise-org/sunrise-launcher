//! Reading a local `.smod` file or repository folder for the "Add from source" sheet, in the background.

use super::*;

impl Launcher {
    /// Add file on Home's Mods tab: opens Add from source and goes straight to its `.smod`
    /// picker, so the file is read, warned about as unreviewed code and installed the way every
    /// local file is.
    /// "Add file": the source sheet, set to a local .smod file. The file picker opens only
    /// once the player presses Continue, after reading the sheet's warning.
    pub(crate) fn add_smod_file(&mut self, _cx: &mut Context<Self>) {
        self.open_add_from_source();
        if matches!(self.github.stage, Stage::Choose(_)) {
            self.github.stage = Stage::Choose(SourceChoice::LocalFile);
        }
    }

    pub(super) fn pick_local_smod(&mut self, cx: &mut Context<Self>) {
        if self.github.target.is_none() || self.github.busy() {
            return;
        }
        let token = self.github.start_picker(PickerKind::LocalFile);
        let pick = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr("Choose a .smod file").into()),
        });
        cx.spawn(async move |this, cx| {
            let outcome: Result<Option<PathBuf>, String> = match pick.await {
                Ok(Ok(paths)) => Ok(paths.and_then(|mut p| p.pop())),
                Ok(Err(e)) => Err(e.to_string()),
                Err(e) => Err(e.to_string()),
            };
            let _ = this.update(cx, |this, cx| {
                if this.ui.sheet != Some(Sheet::GithubAdd) || this.github.target.is_none() {
                    this.github.clear_picker();
                    return;
                }
                if let Some(path) = this
                    .github
                    .finish_picker(token, PickerKind::LocalFile, outcome)
                {
                    this.start_read_local_smod(path);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn start_read_local_smod(&mut self, path: PathBuf) {
        if self.ui.sheet != Some(Sheet::GithubAdd) || self.github.target.is_none() {
            return;
        }
        self.github.clear_picker();
        let (tx, rx) = crate::wake::channel();
        self.github.stage = Stage::ReadingLocal {
            rx,
            from_choose: true,
        };
        std::thread::spawn(move || {
            let result = github_mods::read_local_smod(&path)
                .map(|pkg| LocalRead::File(Box::new(pkg)))
                .map_err(|why| FormError::new(tr("Couldn't open that .smod file."), why));
            let _ = tx.send(result);
        });
    }

    pub(super) fn pick_local_folder(&mut self, cx: &mut Context<Self>) {
        if self.github.target.is_none() || self.github.busy() {
            return;
        }
        let token = self.github.start_picker(PickerKind::LocalFolder);
        let pick = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(tr("Choose a mod folder").into()),
        });
        cx.spawn(async move |this, cx| {
            let outcome: Result<Option<PathBuf>, String> = match pick.await {
                Ok(Ok(paths)) => Ok(paths.and_then(|mut p| p.pop())),
                Ok(Err(e)) => Err(e.to_string()),
                Err(e) => Err(e.to_string()),
            };
            let _ = this.update(cx, |this, cx| {
                if this.ui.sheet != Some(Sheet::GithubAdd) || this.github.target.is_none() {
                    this.github.clear_picker();
                    return;
                }
                if let Some(path) =
                    this.github
                        .finish_picker(token, PickerKind::LocalFolder, outcome)
                {
                    this.start_read_local_folder(path);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn start_read_local_folder(&mut self, path: PathBuf) {
        if self.ui.sheet != Some(Sheet::GithubAdd) || self.github.target.is_none() {
            return;
        }
        self.github.clear_picker();
        let (tx, rx) = crate::wake::channel();
        self.github.stage = Stage::ReadingLocal {
            rx,
            from_choose: false,
        };
        std::thread::spawn(move || {
            let result = github_mods::read_local_folder(&path)
                .map(|packages| LocalRead::Folder(path, packages))
                .map_err(|why| FormError::new(tr("Couldn't read that mod folder."), why));
            let _ = tx.send(result);
        });
    }
}
