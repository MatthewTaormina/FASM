//! `fasm-engine` public library surface — exposed for integration tests and benchmarks.
//!
//! The `run_with_listener` entry point accepts an already-bound `TcpListener`
//! so tests can bind to port 0 and obtain the OS-assigned address.

pub mod admin;
pub mod config;
pub mod dispatcher;
pub mod engine;
pub mod http_handler;
pub mod metrics;
pub mod pubsub;
pub mod queue_looper;
pub mod queues;
pub mod router;
pub mod scheduler;
