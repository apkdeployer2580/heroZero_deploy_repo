#![no_std]
extern crate alloc;

pub mod classes;

use wie_jvm_support::WieJavaClassProto;

// ★ heroZero fork (2026-10-21): +16 classes for 영웅서기 Zero — the offline GCF (`javax.microedition.io`),
// the MMAPI controls (`Control`, `Controllable`, `VolumeControl`), the MIDP 2.0 player
// (`net.wie.MediaPlayer`, `net.wie.PlayerVolumeControl`) and the JSR-120 interfaces.
pub fn get_protos() -> [WieJavaClassProto; 42] {
    [
        classes::javax::microedition::io::Connection::as_proto(),
        classes::javax::microedition::io::ConnectionNotFoundException::as_proto(),
        classes::javax::microedition::io::Connector::as_proto(),
        classes::javax::microedition::io::ContentConnection::as_proto(),
        classes::javax::microedition::io::HttpConnection::as_proto(),
        classes::javax::microedition::io::InputConnection::as_proto(),
        classes::javax::microedition::io::OutputConnection::as_proto(),
        classes::javax::microedition::io::StreamConnection::as_proto(),
        classes::javax::microedition::lcdui::Alert::as_proto(),
        classes::javax::microedition::lcdui::AlertType::as_proto(),
        classes::javax::microedition::lcdui::Canvas::as_proto(),
        classes::javax::microedition::lcdui::ChoiceGroup::as_proto(),
        classes::javax::microedition::lcdui::Command::as_proto(),
        classes::javax::microedition::lcdui::Display::as_proto(),
        classes::javax::microedition::lcdui::Displayable::as_proto(),
        classes::javax::microedition::lcdui::Font::as_proto(),
        classes::javax::microedition::lcdui::Form::as_proto(),
        classes::javax::microedition::lcdui::Graphics::as_proto(),
        classes::javax::microedition::lcdui::Image::as_proto(),
        classes::javax::microedition::lcdui::Item::as_proto(),
        classes::javax::microedition::lcdui::Screen::as_proto(),
        classes::javax::microedition::lcdui::TextBox::as_proto(),
        classes::javax::microedition::lcdui::game::GameCanvas::as_proto(),
        classes::javax::microedition::media::Control::as_proto(),
        classes::javax::microedition::media::Controllable::as_proto(),
        classes::javax::microedition::media::Manager::as_proto(),
        classes::javax::microedition::media::MediaException::as_proto(),
        classes::javax::microedition::media::Player::as_proto(),
        classes::javax::microedition::media::control::VolumeControl::as_proto(),
        classes::javax::microedition::midlet::MIDlet::as_proto(),
        classes::javax::microedition::rms::InvalidRecordIDException::as_proto(),
        classes::javax::microedition::rms::RecordStore::as_proto(),
        classes::javax::microedition::rms::RecordStoreException::as_proto(),
        classes::javax::wireless::messaging::Message::as_proto(),
        classes::javax::wireless::messaging::MessageConnection::as_proto(),
        classes::javax::wireless::messaging::TextMessage::as_proto(),
        classes::net::wie::EventQueue::as_proto(),
        classes::net::wie::Launcher::as_proto(),
        classes::net::wie::MediaPlayer::as_proto(),
        classes::net::wie::PlayerVolumeControl::as_proto(),
        classes::net::wie::SmafPlayer::as_proto(),
        classes::net::wie::WieError::as_proto(),
    ]
}
