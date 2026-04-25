use std::collections::{BTreeMap, BTreeSet};

use crate::actions::ActionsJob;
use crate::error::{CiError, Result};

pub(crate) fn order_jobs(jobs: &[ActionsJob]) -> Result<Vec<ActionsJob>> {
    let map: BTreeMap<_, _> = jobs
        .iter()
        .map(|job| (job.id.clone(), job.clone()))
        .collect();
    let mut ordered = Vec::new();
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();

    for job in jobs {
        visit_job(&map, &job.id, &mut visiting, &mut visited, &mut ordered)?;
    }

    Ok(ordered)
}

fn visit_job(
    map: &BTreeMap<String, ActionsJob>,
    id: &str,
    visiting: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
    ordered: &mut Vec<ActionsJob>,
) -> Result<()> {
    if visited.contains(id) {
        return Ok(());
    }
    if !visiting.insert(id.to_string()) {
        return Err(CiError::Message(format!(
            "cyclic job dependency involving `{id}`"
        )));
    }

    let job = map
        .get(id)
        .ok_or_else(|| CiError::Message(format!("unknown job dependency `{id}`")))?;
    for need in &job.needs {
        visit_job(map, need, visiting, visited, ordered)?;
    }

    visiting.remove(id);
    visited.insert(id.to_string());
    ordered.push(job.clone());
    Ok(())
}
