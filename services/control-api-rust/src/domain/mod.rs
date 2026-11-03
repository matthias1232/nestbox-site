//! Domain vocabulary: the types every other layer is written against.
//!
//! Nothing here performs I/O. That is not a style preference: it is what makes
//! a business rule testable in microseconds and what stops an execution model
//! from leaking into the model of the problem.

pub mod access;
pub mod accounting;
pub mod agent_auth;
pub mod agent_config;
pub mod auth;
pub mod builder;
pub mod chat_turn;
pub mod cudax;
pub mod display;
pub mod egress;
pub mod environment;
pub mod gateway;
pub mod geography;
pub mod job;
pub mod login;
pub mod mcp;
pub mod media;
pub mod metrics;
pub mod model;
pub mod model_grant;
pub mod model_install;
pub mod node;
pub mod node_tunnel;
pub mod openshell_policy;
pub mod pack;
pub mod pack_access;
pub mod pagination;
pub mod persist;
pub mod presence;
pub mod policy;
pub mod privacy;
pub mod privacy_exception;
pub mod privacy_json;
pub mod provider;
pub mod proxy;
pub mod runtime;
pub mod hosting;
pub mod alerts;
pub mod inventory;
pub mod model_cluster;
pub mod mounts;
pub mod permissions;
pub mod search;
pub mod tenancy;
pub mod sandbox;
pub mod ssh_access;
pub mod stack_topology;
pub mod telemetry;

pub use pagination::{Cursor, ListParams, Page, SortDirection};
