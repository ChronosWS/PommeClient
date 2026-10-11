use serde::{Deserialize, Serialize};

/// Represents a logical KeyEvent
#[derive(Serialize, Deserialize)]
pub struct KeyEvent {}

/// Represents a logical MouseEvent
#[derive(Serialize, Deserialize)]
pub struct MouseEvent {}

/// Represents a logical ControllerEvent
#[derive(Serialize, Deserialize)]
pub struct ControllerEvent {}

#[derive(Serialize, Deserialize)]
pub enum InputEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Controller(ControllerEvent),
}

#[derive(Serialize, Deserialize)]
pub struct GameContextId(usize);

#[derive(Serialize, Deserialize)]
pub struct GameContext {
    id: GameContextId,
}
