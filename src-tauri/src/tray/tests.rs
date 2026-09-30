use super::{
    BLUR_GRACE_MS, blurred_recently, click_hides_window, click_opens_menu, needs_attention,
    retry_quit,
};
use crate::error::AppError;
use tauri::tray::{MouseButton, MouseButtonState};

#[test]
fn known_tray_menu_ids_are_stable() {
    // Mutation guard: if these strings change, Windows tray handlers that
    // match on them must change too — and any e2e that simulates a click.
    assert_eq!(super::ID_TOGGLE, "tray-toggle");
    assert_eq!(super::ID_SHOW, "tray-show");
    assert_eq!(super::ID_QUIT, "tray-quit");
}

#[test]
fn right_click_release_opens_the_menu() {
    assert!(click_opens_menu(MouseButton::Right, MouseButtonState::Up));
    assert!(!click_opens_menu(
        MouseButton::Right,
        MouseButtonState::Down
    ));
    assert!(!click_opens_menu(MouseButton::Left, MouseButtonState::Up));
    assert!(!click_opens_menu(MouseButton::Middle, MouseButtonState::Up));
}

#[test]
fn tray_click_hides_only_the_window_the_user_was_looking_at() {
    assert!(click_hides_window(true, true, false));
    // The click itself blurs the window before the button comes up.
    assert!(click_hides_window(true, false, true));
    // Already in the background: bring it forward.
    assert!(!click_hides_window(true, false, false));
    assert!(!click_hides_window(false, false, true));
}

#[test]
fn a_tray_run_that_failed_or_stopped_part_way_is_shown_to_the_user() {
    let refused = AppError::new("journal_unreadable", "the record cannot be read");
    let shown = needs_attention(Err(&refused), true).unwrap();
    assert_eq!(shown.code, "journal_unreadable");
    // A restore that left entries is a failure even though it returned.
    assert_eq!(
        needs_attention(Ok(true), false).unwrap().code,
        "restore_incomplete"
    );
    assert!(needs_attention(Ok(false), false).is_none());
    // Going quiet leaves Quiet Mode on by design.
    assert!(needs_attention(Ok(true), true).is_none());
}

#[test]
fn quit_looks_again_when_the_restore_it_wanted_was_already_running_or_done() {
    assert!(retry_quit(&AppError::new("busy", "")));
    assert!(retry_quit(&AppError::new("not_quiet", "")));
    assert!(!retry_quit(&AppError::new("journal_unreadable", "")));
    assert!(!retry_quit(&AppError::new("platform", "")));
}

#[test]
fn a_blur_counts_only_for_a_short_moment() {
    assert!(!blurred_recently(0, 1_000));
    assert!(blurred_recently(1_000, 1_000 + BLUR_GRACE_MS - 1));
    assert!(!blurred_recently(1_000, 1_000 + BLUR_GRACE_MS));
    assert!(!blurred_recently(5_000, 1_000));
}
