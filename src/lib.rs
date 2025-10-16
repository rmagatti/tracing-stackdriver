#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(not(test), deny(unused_crate_dependencies))]
#![deny(missing_docs, unreachable_pub)]
#![allow(clippy::needless_doctest_main)]
#![doc = include_str!("../README.md")]

// Dummy uses to satisfy unused_crate_dependencies lint
#[cfg(feature = "valuable")]
use http as _;
#[cfg(feature = "opentelemetry")]
use opentelemetry as _;
#[cfg(feature = "opentelemetry")]
use tracing_opentelemetry as _;
#[cfg(feature = "valuable")]
use url as _;
#[cfg(feature = "valuable")]
use valuable as _;
#[cfg(feature = "valuable")]
use valuable_serde as _;

mod event_formatter;
mod google;
mod layer;
mod serializers;
mod visitor;
mod writer;

pub use self::google::*;
pub use self::layer::*;
