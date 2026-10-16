pub mod library;
pub mod memory;
pub mod output;
pub mod pages;
pub mod search;
pub mod storage;
pub mod temporal;

pub type Result<T> = anyhow::Result<T>;
