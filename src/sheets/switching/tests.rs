use super::{build_switch::can_fast_save_choice, receipt::publish_report};
use std::io::{self, Write};
use sunrise_launcher::instances::{Channel, Instance, Kind as InstanceKind};

#[test]
fn can_fast_save_choice_checks_kind_recovery_and_target() {
    let dir = tempfile::tempdir().unwrap();
    let mut instance = Instance {
        id: "own-1".into(),
        name: "Own".into(),
        home: dir.path().to_path_buf(),
        work: dir.path().to_path_buf(),
        version: Some("dev-2026-11-24-abc".into()),
        channel: Channel::Dev,
        kind: InstanceKind::Own,
        group: None,
        icon: None,
        native_profile: Default::default(),
        last_played: None,
        played: Default::default(),
        launch: Default::default(),
        incarnation: None,
    };
    let tag = "dev-2026-11-24-abc";
    // An ordinary profile never played: the choice is only saved, whatever it names and
    // whether or not it is installed (Install downloads it later).
    assert!(can_fast_save_choice(&instance, tag));
    assert!(can_fast_save_choice(&instance, "dev-2026-11-20-old"));
    // Pack profiles never take the fast path.
    instance.kind = InstanceKind::Pack;
    assert!(!can_fast_save_choice(&instance, tag));
    instance.kind = InstanceKind::Own;
    // Pending transaction/bootstrap record -> never take the fast path.
    let txn = dir.path().join("mod-browser/install-txn.json");
    std::fs::create_dir_all(txn.parent().unwrap()).unwrap();
    std::fs::write(&txn, "{}").unwrap();
    assert!(!can_fast_save_choice(&instance, tag));
}

#[test]
fn report_save_can_retry_after_a_partial_write_and_never_clobbers() {
    let folder = tempfile::tempdir().unwrap();
    let report = folder.path().join("report.txt");
    let save = |text: &[u8]| publish_report(folder.path(), &report, |file| file.write_all(text));
    let error = publish_report(folder.path(), &report, |file| {
        file.write_all(b"partial")?;
        Err(io::Error::new(io::ErrorKind::StorageFull, "fixture"))
    })
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::StorageFull);
    assert!(!report.exists());
    save(b"complete report\n").unwrap();
    assert_eq!(
        save(b"other").unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(std::fs::read(&report).unwrap(), b"complete report\n");
}
