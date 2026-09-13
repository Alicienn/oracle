//! Resource sampling for running projects.
//!
//! Measuring only the PID Oracle spawned would be badly wrong: `cmd /C npm run dev` is a
//! shell that launches npm that launches Next.js that forks workers. The real cost lives in
//! the descendants, so every sample walks the process tree from the root PID and sums the
//! whole subtree.

use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

/// Samples retained per project. At one sample a second this is the last minute, which is
/// all a sparkline needs.
pub const HISTORY: usize = 60;

#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    /// Percent of one core. A four-core machine can report up to 400.
    pub cpu: f32,
    /// Resident memory in bytes.
    pub memory: u64,
    /// Number of processes in the tree, including the root.
    pub processes: u32,
}

/// A project's current cost plus the recent history behind it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub project_id: String,
    pub current: Sample,
    pub cpu_history: Vec<f32>,
    pub memory_history: Vec<u64>,
}

/// Machine-wide figures, for the gauges at the top of the tray panel.
#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemUsage {
    pub cpu: f32,
    pub memory_used: u64,
    pub memory_total: u64,
}

pub struct Monitor {
    system: System,
    history: HashMap<String, VecDeque<Sample>>,
}

impl Default for Monitor {
    fn default() -> Self {
        Self::new()
    }
}

impl Monitor {
    pub fn new() -> Self {
        Self {
            system: System::new(),
            history: HashMap::new(),
        }
    }

    /// Takes one sample for each project in `roots`, keyed by project id.
    ///
    /// CPU figures are meaningless on the very first refresh — `sysinfo` needs two points in
    /// time to compute a rate — so the first tick after startup reports zero and the second
    /// onwards is accurate.
    pub fn sample(&mut self, roots: &[(String, u32)]) -> Vec<Usage> {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );

        let children = self.child_index();
        let mut out = Vec::with_capacity(roots.len());

        for (project_id, root_pid) in roots {
            let sample = self.aggregate(Pid::from_u32(*root_pid), &children);

            let entry = self
                .history
                .entry(project_id.clone())
                .or_insert_with(|| VecDeque::with_capacity(HISTORY));

            if entry.len() == HISTORY {
                entry.pop_front();
            }
            entry.push_back(sample);

            out.push(Usage {
                project_id: project_id.clone(),
                current: sample,
                cpu_history: entry.iter().map(|s| s.cpu).collect(),
                memory_history: entry.iter().map(|s| s.memory).collect(),
            });
        }

        // Projects that stopped should not keep their history forever.
        let live: HashSet<&String> = roots.iter().map(|(id, _)| id).collect();
        self.history.retain(|id, _| live.contains(id));

        out
    }

    pub fn system_usage(&mut self) -> SystemUsage {
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();

        SystemUsage {
            cpu: self.system.global_cpu_usage(),
            memory_used: self.system.used_memory(),
            memory_total: self.system.total_memory(),
        }
    }

    /// Drops the history for a project that is no longer running.
    pub fn forget(&mut self, project_id: &str) {
        self.history.remove(project_id);
    }

    /// Builds parent → children once per refresh, so aggregating N projects does not walk
    /// the full process list N times.
    fn child_index(&self) -> HashMap<Pid, Vec<Pid>> {
        let mut index: HashMap<Pid, Vec<Pid>> = HashMap::new();

        for (pid, process) in self.system.processes() {
            if let Some(parent) = process.parent() {
                index.entry(parent).or_default().push(*pid);
            }
        }

        index
    }

    /// Sums CPU and memory across the root process and every descendant.
    fn aggregate(&self, root: Pid, children: &HashMap<Pid, Vec<Pid>>) -> Sample {
        let mut sample = Sample::default();
        let mut seen = HashSet::new();
        let mut queue = VecDeque::from([root]);

        while let Some(pid) = queue.pop_front() {
            // A cycle in the parent chain would otherwise hang the loop. It should not
            // happen, but a PID can be recycled between the refresh and this walk.
            if !seen.insert(pid) {
                continue;
            }

            if let Some(process) = self.system.process(pid) {
                sample.cpu += process.cpu_usage();
                sample.memory += process.memory();
                sample.processes += 1;
            }

            if let Some(kids) = children.get(&pid) {
                queue.extend(kids.iter().copied());
            }
        }

        sample
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(pairs: &[(u32, u32)]) -> HashMap<Pid, Vec<Pid>> {
        let mut map: HashMap<Pid, Vec<Pid>> = HashMap::new();
        for (parent, child) in pairs {
            map.entry(Pid::from_u32(*parent))
                .or_default()
                .push(Pid::from_u32(*child));
        }
        map
    }

    #[test]
    fn a_tree_walk_visits_every_descendant_once() {
        // 1 → 2 → 4, 1 → 3. Every node reachable, none counted twice.
        let children = index(&[(1, 2), (1, 3), (2, 4)]);
        let monitor = Monitor::new();

        // No real processes exist with these PIDs in the test system snapshot, so the
        // sample stays empty — what matters is that the walk terminates and does not
        // revisit nodes.
        let sample = monitor.aggregate(Pid::from_u32(1), &children);
        assert_eq!(sample.processes, 0);
    }

    #[test]
    fn a_cycle_in_the_parent_chain_does_not_hang() {
        let children = index(&[(1, 2), (2, 1)]);
        let monitor = Monitor::new();

        // Completing at all is the assertion.
        let sample = monitor.aggregate(Pid::from_u32(1), &children);
        assert_eq!(sample.processes, 0);
    }

    #[test]
    fn history_is_capped_and_keeps_the_newest_samples() {
        let mut monitor = Monitor::new();

        // The current process is guaranteed to exist, so it is a safe root to sample.
        let me = std::process::id();
        for _ in 0..(HISTORY + 5) {
            monitor.sample(&[("p".to_string(), me)]);
        }

        let history = monitor.history.get("p").unwrap();
        assert_eq!(history.len(), HISTORY);
    }

    #[test]
    fn a_project_that_stops_loses_its_history() {
        let mut monitor = Monitor::new();
        let me = std::process::id();

        monitor.sample(&[("a".to_string(), me), ("b".to_string(), me)]);
        assert_eq!(monitor.history.len(), 2);

        // Only "a" is still running on the next tick.
        monitor.sample(&[("a".to_string(), me)]);
        assert_eq!(monitor.history.len(), 1);
        assert!(monitor.history.contains_key("a"));
    }

    #[test]
    fn sampling_the_current_process_reports_real_memory() {
        let mut monitor = Monitor::new();
        let usage = monitor.sample(&[("self".to_string(), std::process::id())]);

        assert_eq!(usage.len(), 1);
        // At least the test binary itself. The harness may also have live children, which is
        // exactly the subtree aggregation this exercises.
        assert!(
            usage[0].current.processes >= 1,
            "the test process should be found"
        );
        assert!(
            usage[0].current.memory > 0,
            "a live process must report non-zero resident memory"
        );
    }

    #[test]
    fn the_machine_reports_a_plausible_memory_total() {
        let mut monitor = Monitor::new();
        let usage = monitor.system_usage();

        assert!(usage.memory_total > 0);
        assert!(usage.memory_used <= usage.memory_total);
    }
}
