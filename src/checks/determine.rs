//! Extracted helper functions for determine_checks().
//!
//! These functions handle specific responsibilities:
//! - Processing always-run checks
//! - Matching files to check triggers
//! - Handling test discovery
//! - Building CheckToRun instances

#[allow(unused_imports)]
use crate::config::{CheckDefinition, CiConfig};
#[allow(unused_imports)]
use crate::git::ChangedFiles;
#[allow(unused_imports)]
use super::CheckToRun;
#[allow(unused_imports)]
use std::path::Path;

// Placeholder - will be filled in Task 2
