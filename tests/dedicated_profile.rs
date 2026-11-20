//! The dedicated front doors must not run against a profile, even on the same game build.
use std::fs;
use std::path::Path;
use sunrise_launcher::backend;

#[test]
fn server_refuses_profile_roots_before_eula_and_fences_effective_command() {
    let root = tempfile::tempdir().unwrap();
    let game = root.path().join("game");
    let data = root.path().join("launcher");
    let own = game.join("instances/profile");
    let pack = game.join("sunrise-packs/pack/home");
    let server = root.path().join("dedicated");
    for path in [&own, &pack, &server] {
        fs::create_dir_all(path).unwrap();
    }
    std::env::set_var("SUNRISE_HOME", &game);
    std::env::set_var("SUNRISE_LAUNCHER_HOME", data);
    let tag = "nightly-same-build";
    let image = backend::version_dir(tag);
    fs::create_dir_all(backend::resources(tag)).unwrap();
    let client = if cfg!(windows) {
        "sunrise-client.exe"
    } else {
        "sunrise-client"
    };
    fs::write(image.join(client), b"fixture, never executed").unwrap();
    for home in [&game, &own, &pack] {
        assert!(backend::server_command(tag, home).is_err());
        assert!(backend::server(tag, home, true).is_err());
        assert!(!home.join("eula.txt").exists());
    }
    let allowed = backend::server_command(tag, &server).unwrap();
    backend::verify_server_command(tag, &server, &allowed).unwrap();
    assert_eq!(
        allowed.get_current_dir(),
        Some(server.canonicalize().unwrap().as_path())
    );
    assert!(allowed.get_envs().any(|(key, value)| {
        key == "SUNRISE_HOME"
            && value.is_some_and(|value| Path::new(value) == server.canonicalize().unwrap())
    }));
    let mut directory = backend::server_command(tag, &server).unwrap();
    directory.current_dir(&own);
    assert!(backend::verify_server_command(tag, &server, &directory).is_err());
    let mut arguments = backend::server_command(tag, &server).unwrap();
    arguments.arg("--home").arg(&own);
    assert!(backend::verify_server_command(tag, &server, &arguments).is_err());
    let mut environment = backend::server_command(tag, &server).unwrap();
    environment.env("SUNRISE_HOME", &own);
    assert!(backend::verify_server_command(tag, &server, &environment).is_err());
    #[cfg(unix)]
    {
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&own, &alias).unwrap();
        assert!(backend::server_command(tag, &alias).is_err());
    }
}
