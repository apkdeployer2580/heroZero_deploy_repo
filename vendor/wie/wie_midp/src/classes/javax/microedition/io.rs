// ★ heroZero fork (2026-10-21): the MIDP Generic Connection Framework, OFFLINE BY DESIGN.
//
// 영웅서기 Zero only opens two kinds of connection: an HTTP POST of QQ telemetry (`bd`) and an
// `sms://` message for carrier billing (`ep`, through `javax.wireless.messaging`). Neither has a
// server or a carrier behind it any more, so `Connector` refuses every URL with
// `ConnectionNotFoundException` — an `IOException`, which both call sites already catch — and the
// interfaces exist only so the game's classes resolve and verify. No object implementing any of
// them is ever handed out.
mod connection;
mod connection_not_found_exception;
mod connector;
mod content_connection;
mod http_connection;
mod input_connection;
mod output_connection;
mod stream_connection;

pub use self::{
    connection::Connection, connection_not_found_exception::ConnectionNotFoundException, connector::Connector, content_connection::ContentConnection,
    http_connection::HttpConnection, input_connection::InputConnection, output_connection::OutputConnection, stream_connection::StreamConnection,
};
