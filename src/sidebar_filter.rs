//! Filtering, sorting and grouping for the sidebar's session list, kept apart
//! from rendering so each rule can be checked in a unit test against a
//! literal expected output, rather than only by looking at the app.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::hosts::{HostId, Place};

/// The sidebar's Status filter: replaces the old `show_archived` bool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusFilter {
    #[default]
    Active,
    Archived,
    All,
}

impl StatusFilter {
    pub const ALL: [StatusFilter; 3] = [StatusFilter::Active, StatusFilter::Archived, StatusFilter::All];

    pub fn label(self) -> &'static str {
        match self {
            StatusFilter::Active => "Active",
            StatusFilter::Archived => "Archived",
            StatusFilter::All => "All",
        }
    }
}

/// Whether a row passes the Status filter.
pub fn status_matches(status: StatusFilter, archived: bool) -> bool {
    match status {
        StatusFilter::Active => !archived,
        StatusFilter::Archived => archived,
        StatusFilter::All => true,
    }
}

/// Whether a host passes the Where filter; an empty set means "All places".
pub fn where_matches(hosts: &HashSet<HostId>, host: &HostId) -> bool {
    hosts.is_empty() || hosts.contains(host)
}

/// The Where filter's right-aligned value: "All", a single host's name, or "N selected".
pub fn where_label(hosts: &HashSet<HostId>, name_of: impl Fn(&HostId) -> String) -> String {
    match hosts.len() {
        0 => "All".to_string(),
        1 => name_of(hosts.iter().next().expect("len 1")),
        n => format!("{n} selected"),
    }
}

/// How the sidebar's folders are grouped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupBy {
    #[default]
    Folder,
    Where,
    None,
}

impl GroupBy {
    pub const ALL: [GroupBy; 3] = [GroupBy::Folder, GroupBy::Where, GroupBy::None];

    pub fn label(self) -> &'static str {
        match self {
            GroupBy::Folder => "Folder",
            GroupBy::Where => "Where it runs",
            GroupBy::None => "None",
        }
    }
}

/// How the sidebar's sessions are ordered within a folder.
///
/// There's no "Date created" here: an open session's first message gives an
/// honest send time, but a past session (the common case) is only known by
/// its title and last-activity time (`SessionInfo` carries no creation
/// time), so a Date-created sort couldn't be honest for most rows. Per the
/// design brief ("if there's none, leave Date created out and say so"),
/// it's left out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortBy {
    Name,
    /// Matches today's order: open sessions in their existing order, then
    /// past sessions as the agent's history already returns them (newest
    /// first), so this sort is a no-op over that order.
    #[default]
    Activity,
}

impl SortBy {
    pub const ALL: [SortBy; 2] = [SortBy::Name, SortBy::Activity];

    pub fn label(self) -> &'static str {
        match self {
            SortBy::Name => "Name",
            SortBy::Activity => "Last activity",
        }
    }
}

/// Sort by name (case-insensitive, stable); `Activity` leaves the order as given.
pub fn sort_titles<T>(rows: &mut [T], sort: SortBy, title: impl Fn(&T) -> &str) {
    if sort == SortBy::Name {
        rows.sort_by_key(|a| title(a).to_lowercase());
    }
}

/// Order folders for Group by. `Folder` and `None` keep the given order;
/// `Where` clusters folders that share a host, in the host order first seen.
pub fn order_folders(folders: Vec<Place>, group_by: GroupBy) -> Vec<Place> {
    match group_by {
        GroupBy::Folder | GroupBy::None => folders,
        GroupBy::Where => {
            let mut hosts: Vec<HostId> = Vec::new();
            for f in &folders {
                if !hosts.contains(&f.host) {
                    hosts.push(f.host.clone());
                }
            }
            hosts.into_iter().flat_map(|h| folders.iter().filter(move |f| f.host == h).cloned().collect::<Vec<_>>()).collect()
        }
    }
}

/// A case-insensitive substring match on a session's title or its notebook's
/// file name; an empty query matches everything.
pub fn row_matches_search(query: &str, title: &str, notebook_name: Option<&str>) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return true;
    }
    let q = q.to_lowercase();
    title.to_lowercase().contains(&q) || notebook_name.is_some_and(|n| n.to_lowercase().contains(&q))
}

/// The saved sidebar filters (settings.rs holds this on `Settings`).
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SidebarFilters {
    pub status: StatusFilter,
    /// Which hosts to show; empty means "All places".
    pub where_hosts: HashSet<HostId>,
    pub group_by: GroupBy,
    pub sort_by: SortBy,
    pub show_empty_folders: bool,
}

impl SidebarFilters {
    /// Whether anything differs from the defaults (shows "Clear filters").
    pub fn is_default(&self) -> bool {
        self == &SidebarFilters::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(host: HostId, path: &str) -> Place {
        Place { host, path: path.into() }
    }

    #[test]
    fn status_filter_matches() {
        assert!(status_matches(StatusFilter::Active, false));
        assert!(!status_matches(StatusFilter::Active, true));
        assert!(!status_matches(StatusFilter::Archived, false));
        assert!(status_matches(StatusFilter::Archived, true));
        assert!(status_matches(StatusFilter::All, false));
        assert!(status_matches(StatusFilter::All, true));
    }

    #[test]
    fn where_filter_empty_is_all_places() {
        let empty: HashSet<HostId> = HashSet::new();
        assert!(where_matches(&empty, &HostId::ThisMac));
        assert!(where_matches(&empty, &HostId::Server("lab".into())));
    }

    #[test]
    fn where_filter_matches_only_ticked_hosts() {
        let mut hosts = HashSet::new();
        hosts.insert(HostId::Server("lab".into()));
        assert!(!where_matches(&hosts, &HostId::ThisMac));
        assert!(where_matches(&hosts, &HostId::Server("lab".into())));
        assert!(!where_matches(&hosts, &HostId::Server("other".into())));
    }

    #[test]
    fn where_label_counts() {
        let empty: HashSet<HostId> = HashSet::new();
        assert_eq!(where_label(&empty, |_| "?".into()), "All");
        let mut one = HashSet::new();
        one.insert(HostId::ThisMac);
        assert_eq!(where_label(&one, |h| if *h == HostId::ThisMac { "This Mac".into() } else { "?".into() }), "This Mac");
        let mut two = HashSet::new();
        two.insert(HostId::ThisMac);
        two.insert(HostId::Server("lab".into()));
        assert_eq!(where_label(&two, |_| "?".into()), "2 selected");
    }

    #[test]
    fn search_matches_title_or_notebook_case_insensitively() {
        assert!(row_matches_search("resid", "Residual plots", None));
        assert!(row_matches_search("RESID", "Residual plots", None));
        assert!(!row_matches_search("resid", "Normalise Ct values", None));
        assert!(row_matches_search("ct_norm", "Normalise Ct values", Some("ct_normalise.jl")));
        assert!(!row_matches_search("ct_norm", "Growth curves", Some("growth.jl")));
    }

    #[test]
    fn empty_search_matches_everything() {
        assert!(row_matches_search("", "anything", None));
        assert!(row_matches_search("   ", "anything", None));
    }

    #[test]
    fn sort_by_name_is_case_insensitive() {
        let mut rows = vec!["qPCR-2026", "decay-fits", "Growth-curves"];
        sort_titles(&mut rows, SortBy::Name, |s| s);
        assert_eq!(rows, vec!["decay-fits", "Growth-curves", "qPCR-2026"]);
    }

    #[test]
    fn sort_by_activity_keeps_given_order() {
        let mut rows = vec!["b", "a", "c"];
        sort_titles(&mut rows, SortBy::Activity, |s| s);
        assert_eq!(rows, vec!["b", "a", "c"]);
    }

    #[test]
    fn group_by_folder_or_none_keeps_order() {
        let folders = vec![place(HostId::ThisMac, "b"), place(HostId::Server("lab".into()), "a"), place(HostId::ThisMac, "c")];
        assert_eq!(order_folders(folders.clone(), GroupBy::Folder), folders);
        assert_eq!(order_folders(folders.clone(), GroupBy::None), folders);
    }

    #[test]
    fn group_by_where_clusters_by_host_in_first_seen_order() {
        let folders = vec![
            place(HostId::ThisMac, "b"),
            place(HostId::Server("lab".into()), "a"),
            place(HostId::ThisMac, "c"),
            place(HostId::Server("lab".into()), "d"),
        ];
        let grouped = order_folders(folders, GroupBy::Where);
        assert_eq!(
            grouped,
            vec![
                place(HostId::ThisMac, "b"),
                place(HostId::ThisMac, "c"),
                place(HostId::Server("lab".into()), "a"),
                place(HostId::Server("lab".into()), "d"),
            ]
        );
    }

    #[test]
    fn defaults_are_active_all_folder_activity_hidden_empty() {
        let d = SidebarFilters::default();
        assert_eq!(d.status, StatusFilter::Active);
        assert!(d.where_hosts.is_empty());
        assert_eq!(d.group_by, GroupBy::Folder);
        assert_eq!(d.sort_by, SortBy::Activity);
        assert!(!d.show_empty_folders);
        assert!(d.is_default());
    }

    #[test]
    fn any_change_from_defaults_is_not_default() {
        let f = SidebarFilters { status: StatusFilter::All, ..SidebarFilters::default() };
        assert!(!f.is_default());
    }
}
