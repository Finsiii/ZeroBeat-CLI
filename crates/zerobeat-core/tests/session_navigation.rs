use zerobeat_core::{Capability, NavigationState, Route, SessionMode};

#[test]
fn first_run_starts_as_guest_with_every_local_feature() {
    let session = SessionMode::default();

    assert_eq!(session, SessionMode::Guest);
    assert!(session.supports(Capability::Playback));
    assert!(session.supports(Capability::Search));
    assert!(session.supports(Capability::Library));
    assert!(session.supports(Capability::Downloads));
    assert!(session.supports(Capability::Lyrics));
    assert!(!session.supports(Capability::Sync));
}

#[test]
fn login_only_adds_sync_to_guest_capabilities() {
    let guest = SessionMode::Guest;
    let account = SessionMode::Account;

    for capability in [
        Capability::Playback,
        Capability::Search,
        Capability::Library,
        Capability::Downloads,
        Capability::Lyrics,
    ] {
        assert_eq!(guest.supports(capability), account.supports(capability));
    }
    assert!(account.supports(Capability::Sync));
}

#[test]
fn search_state_survives_navigation_away_and_back() {
    let mut navigation = NavigationState::default();

    navigation.open(Route::Search);
    navigation.update_search("tampar");
    navigation.open(Route::Home);
    navigation.open(Route::Search);

    assert_eq!(navigation.active_route(), Route::Search);
    assert_eq!(navigation.search_query(), "tampar");
}

#[test]
fn back_returns_to_the_previous_distinct_route() {
    let mut navigation = NavigationState::default();

    navigation.open(Route::Search);
    navigation.open(Route::Library);
    navigation.back();

    assert_eq!(navigation.active_route(), Route::Search);
}
