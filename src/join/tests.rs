use super::*;
use sunrise_launcher_friends::invite::Invite;

#[test]
fn the_box_tells_an_address_from_an_invite_code() {
    let code = Invite {
        secret: [7; sunrise_launcher_friends::invite::SECRET],
        port: 25565,
        endpoints: vec!["203.0.113.5:25565".parse().unwrap()],
        relay: None,
    }
    .encode();
    assert_eq!(entry(&code), Entry::Invite);
    assert_eq!(
        entry("  play.example.com:25566 "),
        Entry::Address("play.example.com:25566".into())
    );
    assert_eq!(entry("   "), Entry::Empty);
    assert_eq!(entry("a b"), Entry::Unusable);
}

#[test]
fn a_tile_shows_the_first_letter_in_capitals() {
    assert_eq!(initial("  éclair"), "É");
    assert_eq!(initial(""), "?");
}

#[test]
fn the_featured_pair_is_bundled_and_joins_only_with_an_address() {
    assert_eq!(HORIZON_ADDRESS, "play.playhorizon.dev");
    assert_eq!(featured()[0].address, Some("play.playhorizon.dev"));
    let none = featured_with("");
    assert_eq!(none.len(), 2);
    assert_eq!(none[0].name, "Horizon Minecraft");
    assert_eq!(none[1].name, "Sunrise server");
    assert!(none.iter().all(|f| f.address.is_none()));
    assert_eq!(none[0].inert, "Address coming soon");
    let set = featured_with("play.example.org");
    assert_eq!(set[0].address, Some("play.example.org"));
    assert_eq!(set[1].address, None);
}

#[test]
fn hidden_featured_servers_are_kept_beside_the_list_and_come_back() {
    let home = tempfile::tempdir().expect("a scratch folder");
    assert!(hidden::load(home.path()).is_empty());
    let keys: Vec<String> = featured().iter().map(hidden::key).collect();
    assert_eq!(keys, ["play.playhorizon.dev", "sunrise server"]);
    hidden::save(home.path(), &keys).expect("saved");
    assert_eq!(hidden::load(home.path()), keys);
    hidden::save(home.path(), &[]).expect("cleared");
    assert!(hidden::load(home.path()).is_empty());
    assert!(!home.path().join("sunrise-hidden-featured.txt").exists());
}
