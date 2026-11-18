use alloc::{boxed::Box, vec::Vec};

pub type RecordId = u32;

#[async_trait::async_trait]
pub trait Database: Send {
    async fn next_id(&self) -> RecordId;
    /// `None`: the record was not stored. ★ heroZero fork (2026-10-23): it returned an id either
    /// way, so a failed first write of a store reached the MIDlet as a success.
    async fn add(&mut self, data: &[u8]) -> Option<RecordId>;
    async fn get(&self, id: RecordId) -> Option<Vec<u8>>;
    async fn set(&mut self, id: RecordId, data: &[u8]) -> bool;
    async fn delete(&mut self, id: RecordId) -> bool;

    async fn get_record_ids(&self) -> Vec<RecordId>;
}

#[async_trait::async_trait]
pub trait DatabaseRepository {
    async fn open(&self, name: &str, app_id: &str) -> Box<dyn Database>;
    async fn exists(&self, name: &str, app_id: &str) -> bool;
    async fn delete(&self, name: &str, app_id: &str) -> bool;
}
