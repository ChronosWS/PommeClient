//! Application-owned frame response to one routed pause-opening occurrence.

use crate::app::phases::in_game::GameState;
use crate::ui::pause::PauseScreen;

/// Open Main pause only during ordinary gameplay; never toggle or navigate
/// back. Returns whether the caller should refresh cursor grab.
pub fn open_pause(game: &mut GameState) -> bool {
    if !pause_allowed(game) {
        return false;
    }
    game.paused = true;
    game.pause_screen = PauseScreen::Main;
    true
}

pub fn pause_allowed(game: &GameState) -> bool {
    pause_eligible(
        game.input_live(),
        game.dead,
        game.death_screen_open,
        game.options_from_game,
        game.chunk_load_bench.is_some(),
    )
}

fn pause_eligible(
    input_live: bool,
    dead: bool,
    death_screen_open: bool,
    options_from_game: bool,
    chunk_load_bench: bool,
) -> bool {
    input_live && !dead && !death_screen_open && !options_from_game && !chunk_load_bench
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_requires_live_ordinary_gameplay() {
        assert!(pause_eligible(true, false, false, false, false));
        assert!(!pause_eligible(false, false, false, false, false));
        assert!(!pause_eligible(true, true, false, false, false));
        assert!(!pause_eligible(true, false, true, false, false));
        assert!(!pause_eligible(true, false, false, true, false));
        assert!(!pause_eligible(true, false, false, false, true));
    }
}
