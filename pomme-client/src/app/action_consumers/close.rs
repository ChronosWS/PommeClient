//! Application-owned frame response to one routed cancellation occurrence.

use crate::app::core::AppCore;
use crate::app::phases::in_game::{GameState, settle_server_dialog};
use crate::net::connection::ConnectionHandle;
use crate::ui::death;
use crate::ui::pause::PauseScreen;

/// Cancel the topmost screen without opening pause. Returns whether the caller
/// should refresh cursor grab; text-stream invalidation belongs to the caller.
pub fn close_game(core: &mut AppCore, game: &mut GameState, connection: &ConnectionHandle) -> bool {
    if game.options_from_game {
        core.input.request_menu_close();
        return false;
    }
    if game.chat.has_pending_modal_prompt() {
        return game.escape_chat();
    }
    if let Some(dialog) = game.server_dialog.as_mut() {
        let action = dialog.handle_escape();
        settle_server_dialog(action, core, connection, game);
        return true;
    }
    // Options and dialog overlays take precedence, but a blocking death screen
    // must not let cancellation reach any underlying gameplay screen.
    if game.dead || game.death_screen_open {
        // Vanilla's death screen cannot cancel, but its delayed "Really Quit"
        // confirmation can: the negative/cancel response respawns the player.
        if death_respawn_allowed(game.death_confirm, game.death_confirm_ticks) {
            game.death_confirm = false;
            core.send_respawn(connection, game);
        }
        return false;
    }
    if game.benchmark_result.is_some() {
        game.benchmark_result = None;
        game.benchmark_upload = None;
        return true;
    }
    if game.chunk_load_result.is_some() {
        game.chunk_load_result = None;
        game.chunk_load_upload = None;
        return true;
    }
    if game.chat.is_open() {
        return game.escape_chat();
    }
    if game.game_mode_switcher.is_some() {
        game.game_mode_switcher = None;
        return true;
    }
    if game.chunk_load_bench.is_some() {
        // update_game restores the render distance after cancellation.
        game.chunk_load_abort = true;
        return true;
    }
    if game.creative_inventory_open {
        game.close_creative_inventory();
        return true;
    }
    if game.inventory_open || game.open_container.is_some() {
        game.close_menu();
        return true;
    }
    close_pause(&mut game.paused, &mut game.pause_screen)
}

fn death_respawn_allowed(confirm: bool, ticks: u32) -> bool {
    confirm && death::buttons_ready(ticks)
}

fn close_pause(paused: &mut bool, screen: &mut PauseScreen) -> bool {
    if !*paused {
        return false;
    }
    match screen {
        PauseScreen::ChunkLoader => *screen = PauseScreen::Benchmark,
        PauseScreen::Benchmark => *screen = PauseScreen::Main,
        PauseScreen::Main | PauseScreen::Hidden => *paused = false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_submenus_back_out_one_screen_per_occurrence_then_resume() {
        let mut paused = true;
        let mut screen = PauseScreen::ChunkLoader;
        assert!(close_pause(&mut paused, &mut screen));
        assert!(paused);
        assert!(screen == PauseScreen::Benchmark);
        assert!(close_pause(&mut paused, &mut screen));
        assert!(paused);
        assert!(screen == PauseScreen::Main);
        assert!(close_pause(&mut paused, &mut screen));
        assert!(!paused);
        assert!(!close_pause(&mut paused, &mut screen));
    }

    #[test]
    fn hidden_pause_resumes_but_cancellation_never_opens_pause() {
        let mut paused = true;
        let mut screen = PauseScreen::Hidden;
        assert!(close_pause(&mut paused, &mut screen));
        assert!(!paused);
        for mut screen in [
            PauseScreen::Main,
            PauseScreen::Benchmark,
            PauseScreen::ChunkLoader,
            PauseScreen::Hidden,
        ] {
            let original = screen;
            assert!(!close_pause(&mut paused, &mut screen));
            assert!(!paused);
            assert!(screen == original);
        }
    }

    #[test]
    fn main_death_screen_ignores_cancellation_and_confirm_obeys_delay() {
        assert!(!death_respawn_allowed(false, u32::MAX));
        assert!(!death_respawn_allowed(true, 0));
        let ready = (0..=100)
            .find(|&ticks| death::buttons_ready(ticks))
            .expect("death buttons become ready");
        assert!(ready > 0);
        assert!(!death_respawn_allowed(true, ready - 1));
        assert!(death_respawn_allowed(true, ready));
        assert!(death_respawn_allowed(true, u32::MAX));
    }
}
