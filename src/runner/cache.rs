use std::fs;

use crate::error::Result;
use crate::git::sanitize_component;
use crate::runner::AppContext;

use super::file_system::copy_recursively;

#[derive(Default)]
pub(crate) struct CacheState {
    pub(crate) pending: Vec<PendingCache>,
}

pub(crate) struct PendingCache {
    pub(crate) key: String,
    pub(crate) paths: Vec<String>,
}

pub(crate) fn restore_cache(ctx: &AppContext, key: &str, paths: &[String]) -> Result<()> {
    let cache_root = ctx
        .repo
        .state_dir
        .join("cache")
        .join(sanitize_component(key));
    if !cache_root.exists() {
        return Ok(());
    }

    for path in paths {
        let target = ctx.repo.root.join(path);
        let source = cache_root.join(path);
        if source.exists() {
            copy_recursively(&source, &target)?;
        }
    }
    Ok(())
}

pub(crate) fn save_pending_caches(ctx: &AppContext, cache_state: &CacheState) -> Result<()> {
    for pending in &cache_state.pending {
        let cache_root = ctx
            .repo
            .state_dir
            .join("cache")
            .join(sanitize_component(&pending.key));
        fs::create_dir_all(&cache_root)?;
        for path in &pending.paths {
            let source = ctx.repo.root.join(path);
            if source.exists() {
                copy_recursively(&source, &cache_root.join(path))?;
            }
        }
    }
    Ok(())
}
