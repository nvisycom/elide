#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod lingua_detector;
mod lingua_enricher;

pub use self::lingua_enricher::LinguaEnricher;
