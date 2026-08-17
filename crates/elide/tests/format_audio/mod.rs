//! Audio-format scenarios, one module per format. They share the single
//! audio handler and STT enrichment; shared helpers live here as the
//! family grows.
#![allow(dead_code)]

mod mp3;
mod wav;
