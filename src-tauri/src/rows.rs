//! The process picker's rows: every instance of a program folded into one
//! line, heaviest first, so a user can see what is worth parking.

use cq_core::ProcessInfo;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ProcessRow {
    pub name: String,
    pub instances: usize,
    pub memory_bytes: u64,
    pub cpu_percent: f32,
    pub exe: Option<String>,
}

pub fn fold_processes(processes: Vec<ProcessInfo>) -> Vec<ProcessRow> {
    let mut rows: std::collections::BTreeMap<String, ProcessRow> = Default::default();
    for process in processes {
        let key = cq_core::policy::normalize(&process.name);
        if key.is_empty() {
            continue;
        }
        let row = rows.entry(key).or_insert_with(|| ProcessRow {
            name: process.name.clone(),
            instances: 0,
            memory_bytes: 0,
            cpu_percent: 0.0,
            exe: process.exe.as_ref().map(|p| p.display().to_string()),
        });
        row.instances += 1;
        row.memory_bytes += process.memory_bytes;
        row.cpu_percent += process.cpu_percent;
        if row.exe.is_none() {
            row.exe = process.exe.as_ref().map(|p| p.display().to_string());
        }
    }
    let mut list: Vec<ProcessRow> = rows.into_values().collect();
    list.sort_by_key(|row| std::cmp::Reverse(row.memory_bytes));
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(pid: u32, name: &str, memory: u64) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: name.into(),
            exe: None,
            args: vec![],
            cwd: None,
            memory_bytes: memory,
            cpu_percent: 1.0,
            start_time: 0,
            parent: None,
        }
    }

    #[test]
    fn instances_fold_by_normalised_name_and_sort_heaviest_first() {
        let rows = fold_processes(vec![
            process(1, "chrome.exe", 100),
            process(2, "Chrome.exe", 300),
            process(3, "game.exe", 900),
            process(4, "", 5),
        ]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "game.exe");
        assert_eq!(rows[1].instances, 2);
        assert_eq!(rows[1].memory_bytes, 400);
        assert_eq!(rows[1].cpu_percent, 2.0);
    }
}
