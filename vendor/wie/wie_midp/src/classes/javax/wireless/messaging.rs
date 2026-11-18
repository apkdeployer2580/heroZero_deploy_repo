// ★ heroZero fork (2026-10-21): JSR-120 (WMA) interfaces, declared so the game's `ep` (carrier
// billing over `sms://`) resolves and verifies. Interfaces only, by design: a `MessageConnection`
// can only come from `javax.microedition.io.Connector.open`, which refuses every URL in this
// offline build, so nothing implementing these is ever handed out.
mod message;
mod message_connection;
mod text_message;

pub use self::{message::Message, message_connection::MessageConnection, text_message::TextMessage};
