//! Device drivers. Each one turns hardware traffic into [`model::Event`]s sent
//! on a tokio channel, and stops when the receiving side is dropped.

pub mod csafe;
pub mod h10;
pub mod pm5;
pub mod sim;
pub mod sim_pose;
