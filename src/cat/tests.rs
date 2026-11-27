use super::*;

fn opaque(px: &Pixels) -> usize {
    px.0.as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[3] == 255)
        .count()
}

#[test]
fn the_cat_hides_comes_out_into_the_room_it_has_and_goes_back() {
    let mut outing = Outing::new(&TABBY_WAYS, 7, 1000.);
    outing.step(0., 400., false);
    assert!(outing.view(0.).is_none(), "it starts out of sight");
    let (mut now, mut furthest, mut visited) = (0., 0f32, false);
    // Until it has come out and is away again.
    while !(visited && matches!(outing.scene, Scene::Away { .. })) {
        now += 40.;
        assert!(now < 600_000., "it went home within ten minutes");
        outing.step(now, 400., false);
        if let Some(view) = outing.view(now) {
            furthest = furthest.max(view.out);
            visited |= view.pettable;
        }
    }
    assert!(
        furthest > WIDTH && furthest <= 400.,
        "wholly out, and never past the room: {furthest}"
    );
    assert!(outing.view(now).is_none());
}

#[test]
fn with_too_little_room_the_cat_only_peeks_and_with_none_it_stays_in() {
    let mut outing = Outing::new(&SIAMESE_WAYS, 3, 0.);
    for step in 0..200 {
        outing.step(step as f32 * 40., 0., false);
        assert!(outing.view(step as f32 * 40.).is_none());
    }
    let mut furthest = 0f32;
    for step in 200..2000 {
        let now = step as f32 * 40.;
        outing.step(now, 60., false);
        if let Some(view) = outing.view(now) {
            assert!(!view.pettable);
            furthest = furthest.max(view.out);
        }
    }
    assert!(
        furthest > PEEK_OUT - 1. && furthest <= PEEK_OUT,
        "head and chest, no further: {furthest}"
    );
}

#[test]
fn on_play_the_cat_sleeps_without_frames_and_drops_off_when_play_is_needed() {
    let mut outing = Outing::new(&TABBY_WAYS, 7, 0.);
    outing.perch_next = true;
    // Play is busy: it does not climb up.
    outing.step(0., 400., false);
    assert!(!matches!(outing.scene, Scene::Perch { .. }));
    let mut outing = Outing::new(&TABBY_WAYS, 7, 0.);
    outing.perch_next = true;
    outing.step(0., 0., true);
    let mut now = 0.;
    while now < CLIMB + CURL + 40. {
        now += 40.;
        outing.step(now, 0., true);
    }
    let view = outing.view(now).expect("it is on Play");
    assert_eq!(view.perch, Some(1.), "wholly up");
    assert!(
        !view.pettable,
        "a picture only: clicks are Play's and the page's"
    );
    assert_eq!(view.pose, rest(State::Sleep), "and curled up asleep");
    let wait = outing.next_change(now);
    assert!(wait >= Duration::from_secs(40), "no frames while it sleeps");
    assert_eq!(outing.view(now + 30_000.).map(|v| v.pose), Some(view.pose));
    // Play is pressed, or turns busy: off it goes, and it is gone within the drop.
    outing.step(now + 30_000., 0., false);
    assert!(matches!(outing.scene, Scene::Drop { .. }));
    outing.step(now + 30_000. + DROP, 0., false);
    assert!(outing.view(now + 30_000. + DROP).is_none());
}

#[test]
fn a_resting_cat_asks_for_no_frames_until_it_next_does_something() {
    let mut cat = Cat::new(&TABBY_WAYS, 7);
    cat.advance(0.);
    let next = cat.next_change().expect("a resting cat waits");
    assert!(
        next >= 4000.,
        "the first rest or blink is seconds away: {next}"
    );
    assert_eq!(
        cat.pose(0.),
        cat.pose(next - 1.),
        "and it holds still until then"
    );
    cat.advance(next);
    assert!(cat.next_change().is_none(), "then it moves, frame by frame");
}

#[test]
fn petting_wakes_a_meow_and_the_cat_settles_back_where_it_was() {
    let mut cat = Cat::new(&SIAMESE_WAYS, 3);
    cat.pet(100.);
    assert_eq!(cat.action().map(|(a, _)| a.name), Some("meow"));
    let loud = cat.pose(100. + 480.);
    assert!(
        loud[MOUTH] > 0.9 && loud[TURN] > 0.9,
        "mouth wide, facing you"
    );
    cat.advance(100. + 2701.);
    assert!(cat.action().is_none());
    assert_eq!(cat.pose(3000.), rest(State::Sit));
}

#[test]
fn a_cat_asleep_in_the_bar_hisses_when_petted_and_stays_asleep_after() {
    // In the bar a visit can end in a nap (`fall asleep`); on Play the cat is never petted.
    let mut cat = Cat::new(&TABBY_WAYS, 5);
    cat.state = State::Sleep;
    cat.pet(0.);
    assert_eq!(cat.action().map(|(a, _)| a.name), Some("grumpy"));
    assert!(cat.pose(700. * TABBY_WAYS.pace)[ANGER] > 0.9);
    cat.advance(2800. * TABBY_WAYS.pace + 1.);
    assert_eq!(cat.state, State::Sleep);
}

#[test]
fn the_cat_stays_away_when_switched_off_or_under_reduced_motion() {
    assert!(cat_out(true, false));
    assert!(!cat_out(false, false), "switched off");
    assert!(!cat_out(true, true), "reduced motion");
    assert!(!cat_out(false, true));
    // Staying away is the same state either way: nothing kept, no wake due.
    let mut state = CatState {
        cat: Some(Outing::new(&TABBY_WAYS, 7, 0.)),
        seen: Some(Instant::now()),
        wake: Some(Instant::now() + Duration::from_millis(40)),
        ..CatState::default()
    };
    assert!(state.switch_off().is_none(), "no picture was drawn yet");
    assert!(state.cat.is_none() && state.seen.is_none() && state.wake.is_none());
}

#[test]
fn a_cat_switched_off_keeps_nothing_and_schedules_nothing() {
    let mut state = CatState {
        cat: Some(Outing::new(&SIAMESE_WAYS, 1, 0.)),
        seen: Some(Instant::now()),
        wake: Some(Instant::now() + Duration::from_secs(5)),
        shown: Some((
            rest(State::Sit),
            1,
            false,
            Arc::new(RenderImage::new([image::Frame::new(
                image::RgbaImage::new(1, 1),
            )])),
        )),
        ..CatState::default()
    };
    assert!(state.switch_off().is_some());
    assert!(state.cat.is_none() && state.seen.is_none() && state.wake.is_none());
    assert!(state.shown.is_none());
    assert!(state.switch_off().is_none());
}

#[test]
fn each_coat_draws_a_whole_cat_with_its_own_eyes() {
    for (coat, eye) in [(&SIAMESE, [255, 178, 92]), (&TABBY, [78, 214, 196])] {
        let px = picture(coat, &SIT);
        assert!(opaque(&px) > 500, "a body, not a few pixels");
        assert!(
            px.0.as_chunks::<4>().0.iter().any(|p| p[..3] == eye),
            "its eye colour, as BGR"
        );
    }
}

#[test]
fn every_action_ends_where_its_state_rests() {
    for (i, action) in actions().iter().enumerate() {
        let mut cat = Cat::new(&SIAMESE_WAYS, 9);
        cat.state = action.from;
        cat.playing = Some((Playing::Own(i), 0.));
        let end = action.keys.last().map_or(0., Key::at);
        let to = action.to.unwrap_or(action.from);
        let (ends, rests) = (cat.pose(end), rest(to));
        assert!(
            ends.iter().zip(rests).all(|(a, b)| (a - b).abs() < 1e-4),
            "{} ends at rest",
            action.name
        );
    }
}

mod talk {
    use super::super::talk::*;

    fn click(talk: Talk, now: f32, roll: f32) -> (Said, Talk) {
        next_line(talk, now, roll)
    }

    #[test]
    fn a_first_click_gets_a_random_line_and_never_the_same_one_twice_in_a_row() {
        let mut talk = Talk::default();
        let mut last = "";
        // Clicks far apart, so none escalates, with every roll there is.
        for i in 0..400 {
            let roll = (i % 20) as f32 / 20.;
            let (said, next) = click(talk, i as f32 * 10_000., roll);
            assert!(LINES.contains(&said.line) && !said.leaves);
            assert_ne!(said.line, last);
            last = said.line;
            talk = next;
        }
    }

    #[test]
    fn every_random_line_can_come_up() {
        let mut seen = std::collections::HashSet::new();
        for i in 0..20 {
            let (said, _) = click(Talk::default(), 0., i as f32 / 20.);
            seen.insert(said.line);
        }
        assert_eq!(seen.len(), LINES.len());
    }

    #[test]
    fn every_escalation_has_four_distinct_steps_and_the_last_one_leaves() {
        let mut all = std::collections::HashSet::new();
        for (n, steps) in ESCALATIONS.iter().enumerate() {
            let own: std::collections::HashSet<_> = steps.iter().collect();
            assert_eq!(own.len(), steps.len(), "sequence {n} repeats a step");
            for line in steps {
                assert!(!LINES.contains(line), "{line} is also a first-click line");
                assert!(all.insert(*line), "{line} is in two sequences");
            }
        }
    }

    #[test]
    fn quick_clicks_run_the_picked_escalation_in_order_and_the_last_one_leaves() {
        for (n, steps) in ESCALATIONS.iter().enumerate() {
            // The roll that picks sequence n.
            let pick = (n as f32 + 0.5) / ESCALATIONS.len() as f32;
            let (first, mut talk) = click(Talk::default(), 0., 0.3);
            assert!(LINES.contains(&first.line));
            for (i, want) in steps.iter().enumerate() {
                // Later rolls do not change the sequence it is in.
                let roll = if i == 0 { pick } else { 0.99 };
                let (said, next) = click(talk, 1000. * (i + 1) as f32, roll);
                assert_eq!(said.line, *want);
                assert_eq!(said.leaves, i == steps.len() - 1);
                talk = next;
            }
            // It left: the next click, however soon, starts over with a random line.
            let (said, _) = click(talk, 5500., 0.5);
            assert!(LINES.contains(&said.line));
        }
    }

    #[test]
    fn every_escalation_can_be_picked() {
        let mut seen = std::collections::HashSet::new();
        for i in 0..20 {
            let (_, talk) = click(Talk::default(), 0., 0.1);
            let (said, _) = click(talk, 1000., i as f32 / 20.);
            seen.insert(said.line);
        }
        assert_eq!(seen.len(), ESCALATIONS.len());
    }

    #[test]
    fn a_pause_resets_the_escalation() {
        let first = ESCALATIONS[0];
        let (_, talk) = click(Talk::default(), 0., 0.1);
        let (said, talk) = click(talk, 1000., 0.);
        assert_eq!(said.line, first[0]);
        let (said, talk) = click(talk, 2000., 0.9);
        assert_eq!(said.line, first[1]);
        // More than four seconds of peace.
        let (said, talk) = click(talk, 7000., 0.1);
        assert!(LINES.contains(&said.line));
        let (said, _) = click(talk, 8000., 0.);
        assert_eq!(said.line, first[0]);
    }

    #[test]
    fn a_longer_line_stays_longer_up_to_a_cap() {
        assert_eq!(lasts(0), 3500.);
        assert!(lasts(40) > lasts(10));
        assert_eq!(lasts(500), 7000.);
        for line in LINES.iter().chain(ESCALATIONS.iter().flatten()) {
            assert!(lasts(line.chars().count()) >= 3500.);
        }
    }

    #[test]
    fn the_bubble_fades_in_holds_fades_out_and_is_plain_when_motion_is_reduced() {
        let shown = lasts(20);
        assert_eq!(bubble_alpha(0., shown, false), Some(0.));
        assert_eq!(bubble_alpha(1500., shown, false), Some(1.));
        assert!(bubble_alpha(shown - 100., shown, false).is_some_and(|a| a > 0. && a < 1.));
        assert_eq!(bubble_alpha(shown, shown, false), None);
        assert_eq!(bubble_alpha(10., shown, true), Some(1.));
        assert_eq!(bubble_alpha(shown + 1., shown, true), None);
    }

    #[test]
    fn clicking_out_the_escalation_sends_the_cat_home_for_a_while() {
        use super::super::*;
        let mut outing = Outing::new(&TABBY_WAYS, 5, 0.);
        outing.scene = Scene::Visit {
            at: 200.,
            until: 1e9,
        };
        for i in 0..5 {
            outing.speak(i as f32 * 1000.);
        }
        assert!(outing.bubble.is_some());
        let mut now = 5000.;
        while !matches!(outing.scene, Scene::Away { .. }) {
            now += 40.;
            assert!(now < 60_000., "it walked off");
            outing.step(now, 400., false);
        }
        assert!(matches!(outing.scene, Scene::Away { until } if until - now >= 19_000.));
    }
}
