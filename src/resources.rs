//! A cluster job's resources as the app edits them: in the cluster dialog
//! (its defaults) and in the new-session screen's resources chip (one
//! session's). Presets, a partition, and steppers for CPUs, memory and the
//! time limit, kept within the partition's limits.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use wire::slurm::{PRESETS, Partition, Resources, duration_text};

use crate::new_session::{Glyph, glyph, menu_row};
use crate::{Workspace, theme};

const CPUS: [u32; 12] = [1, 2, 4, 8, 12, 16, 24, 32, 48, 64, 96, 128];
const MEM_GB: [u32; 14] = [1, 2, 4, 8, 16, 32, 48, 64, 96, 128, 192, 256, 384, 512];
const MINUTES: [u32; 12] = [15, 30, 60, 120, 240, 480, 720, 1440, 2880, 4320, 7200, 10080];

/// Which resources an editor changes.
#[derive(Clone, Copy, PartialEq)]
pub enum Target {
    /// The cluster dialog's defaults.
    Dialog,
    /// The new-session screen's.
    Draft,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Field {
    Cpus,
    Mem,
    Time,
}

#[derive(Clone)]
pub enum Change {
    Preset(usize),
    Partition(Option<String>),
    Step(Field, bool),
    TogglePartitions,
}

/// The next value up or down `ladder` from `value`, at most `max`.
fn step_on(ladder: &[u32], value: u32, up: bool, max: Option<u32>) -> u32 {
    let next = if up {
        ladder.iter().copied().find(|&v| v > value).unwrap_or(value)
    } else {
        ladder.iter().rev().copied().find(|&v| v < value).unwrap_or(value)
    };
    match max {
        Some(max) if next > max => max.max(value.min(max)),
        _ => next,
    }
}

/// Apply `change` to `r`, keeping within `partitions`' limits.
pub fn apply(r: &mut Resources, change: &Change, partitions: &[Partition]) {
    let find = |name: Option<&str>| match name {
        Some(name) => partitions.iter().find(|p| p.name == name),
        None => partitions.iter().find(|p| p.default),
    };
    match change {
        Change::Preset(i) => *r = r.sized_as(*i),
        Change::Partition(name) => r.partition = name.clone(),
        Change::Step(field, up) => {
            let p = find(r.partition.as_deref());
            match field {
                Field::Cpus => r.cpus = step_on(&CPUS, r.cpus, *up, p.map(|p| p.cpus).filter(|&c| c > 0)),
                Field::Mem => r.mem_gb = step_on(&MEM_GB, r.mem_gb, *up, p.map(|p| p.mem_gb()).filter(|&m| m > 0)),
                Field::Time => r.minutes = step_on(&MINUTES, r.minutes, *up, p.and_then(|p| p.max_minutes)),
            }
        }
        Change::TogglePartitions => {}
    }
    r.clip(find(r.partition.as_deref()));
}

/// Whether `r` is what picking preset `i` would give now: its size, capped to
/// the partition's limits.
fn is_preset(r: &Resources, i: usize, partitions: &[Partition]) -> bool {
    let mut picked = r.clone();
    apply(&mut picked, &Change::Preset(i), partitions);
    (picked.cpus, picked.mem_gb, picked.minutes) == (r.cpus, r.mem_gb, r.minutes)
}

impl Workspace {
    fn resources_target(&mut self, target: Target) -> Option<(&mut Resources, Vec<Partition>, &mut bool)> {
        match target {
            Target::Dialog => self.server_dialog.as_mut().and_then(|d| d.cluster.as_mut()).map(|c| (&mut c.resources, c.partitions.clone(), &mut c.partition_menu)),
            Target::Draft => {
                let partitions = self.draft_cluster().map(|c| c.partitions.clone()).unwrap_or_default();
                let draft = &mut self.draft;
                let resources = draft.resources.as_mut()?;
                Some((resources, partitions, &mut draft.partition_menu))
            }
        }
    }

    pub fn change_resources(&mut self, target: Target, change: Change, cx: &mut Context<Self>) {
        let Some((resources, partitions, menu)) = self.resources_target(target) else { return };
        if matches!(change, Change::TogglePartitions) {
            *menu = !*menu;
        } else {
            if matches!(change, Change::Partition(_)) {
                *menu = false;
            }
            apply(resources, &change, &partitions);
        }
        cx.notify();
    }

    /// Presets (for a session's own), partition, CPUs, memory and time limit.
    pub fn resource_rows(&self, target: Target, r: &Resources, partitions: &[Partition], menu_open: bool, presets: bool, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let on = move |change: Change| move |this: &mut Workspace, _: &ClickEvent, _: &mut Window, cx: &mut Context<Workspace>| this.change_resources(target, change.clone(), cx);
        let mut rows = Vec::new();
        if presets {
            let buttons = PRESETS.iter().enumerate().map(|(i, (name, ..))| {
                let active = is_preset(r, i, partitions);
                div()
                    .id(("preset", i))
                    .role(Role::Button)
                    .flex_1()
                    .h(px(26.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.))
                    .border_1()
                    .border_color(if active { theme::accent() } else { theme::composer_edge() })
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::bg_card()))
                    .text_size(theme::size_meta())
                    .child(*name)
                    .on_click(cx.listener(on(Change::Preset(i))))
            });
            rows.push(label("Presets").into_any_element());
            rows.push(div().flex().gap(px(6.)).pb(px(6.)).children(buttons).into_any_element());
        }
        let default_name = partitions.iter().find(|p| p.default).map(|p| p.name.clone());
        let shown = r.partition.clone().or_else(|| default_name.clone().map(|n| format!("{n} (default)"))).unwrap_or_else(|| "Cluster default".into());
        let partition = div()
            .id(("partition-select", target as usize))
            .w(px(136.))
            .h(px(26.))
            .px(px(10.))
            .flex()
            .items_center()
            .justify_between()
            .rounded(px(5.))
            .border_1()
            .border_color(theme::composer_edge())
            .cursor_pointer()
            .child(div().overflow_hidden().whitespace_nowrap().text_ellipsis().child(shown))
            .child(glyph(Glyph::Chevron, theme::text_faint()))
            .on_click(cx.listener(on(Change::TogglePartitions)));
        rows.push(row("Partition", partition).into_any_element());
        if menu_open {
            let options = std::iter::once((None, "Cluster default".to_owned(), None))
                .chain(partitions.iter().map(|p| (Some(p.name.clone()), p.name.clone(), Some(p))))
                .enumerate()
                .map(|(i, (value, name, p))| {
                    let limits = p.map(|p| {
                        let time = p.max_minutes.map_or("no time limit".into(), |m| format!("up to {}", duration_text(m)));
                        format!("{time} · {} CPUs · {} GB per node", p.cpus, p.mem_gb())
                    });
                    menu_row(("partition-option", i), r.partition == value, false)
                        .child(div().flex_shrink_0().child(name))
                        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_right().text_size(theme::size_meta_small()).text_color(theme::text_faint()).children(limits))
                        .on_click(cx.listener(on(Change::Partition(value))))
                });
            let options: Vec<_> = options.collect();
            rows.push(
                div()
                    .mb(px(4.))
                    .p(px(4.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(theme::composer_edge())
                    .bg(theme::bg_card())
                    .when(partitions.is_empty(), |d| d.child(div().px(px(8.)).py(px(4.)).text_size(theme::size_meta()).text_color(theme::text_faint()).child("Test connection lists this cluster's partitions.")))
                    .children(options)
                    .into_any_element(),
            );
        }
        let steppers = [
            ("CPUs", Field::Cpus, r.cpus.to_string()),
            ("Memory", Field::Mem, if r.mem_gb == 0 { "per CPU".into() } else { format!("{} GB", r.mem_gb) }),
            ("Time limit", Field::Time, duration_text(r.minutes)),
        ];
        let steppers = steppers.map(|(name, field, value)| (name, stepper(field as usize + 10 * target as usize, value, cx.listener(on(Change::Step(field, false))), cx.listener(on(Change::Step(field, true))))));
        match target {
            // The dialog has more to fit, so its three share one row.
            Target::Dialog => rows.push(
                div()
                    .pt(px(4.))
                    .flex()
                    .gap(px(12.))
                    .children(steppers.map(|(name, stepper)| div().flex_1().flex().flex_col().gap(px(4.)).child(label(name).pb_0()).child(stepper.w_full())))
                    .into_any_element(),
            ),
            Target::Draft => rows.extend(steppers.map(|(name, stepper)| row(name, stepper).into_any_element())),
        }
        rows
    }
}

fn label(text: &'static str) -> Div {
    div().pb(px(4.)).text_size(theme::size_meta_small()).text_color(theme::text_faint()).child(text)
}

fn row(name: &'static str, control: impl IntoElement) -> Div {
    div().h(px(34.)).flex().items_center().justify_between().gap(px(12.)).child(div().text_color(theme::text_secondary()).child(name)).child(control)
}

type OnClick = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// − value +
fn stepper(id: usize, value: String, minus: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static, plus: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Div {
    let button = |id: (&'static str, usize), text: &'static str, handler: OnClick| {
        div()
            .id(id)
            .role(Role::Button)
            .w(px(28.))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .text_color(theme::text_muted())
            .hover(|s| s.bg(theme::bg_raised()).text_color(theme::text_primary()))
            .child(text)
            .on_click(handler)
    };
    div()
        .w(px(136.))
        .h(px(26.))
        .flex()
        .items_center()
        .rounded(px(5.))
        .border_1()
        .border_color(theme::composer_edge())
        .overflow_hidden()
        .child(button(("step-down", id), "−", Box::new(minus)))
        .child(div().flex_1().h_full().flex().items_center().justify_center().border_l_1().border_r_1().border_color(theme::composer_edge()).child(value))
        .child(button(("step-up", id), "+", Box::new(plus)))
}

#[cfg(test)]
mod tests {
    use super::{Change, Field, apply, is_preset};
    use wire::slurm::{Partition, Resources};

    fn short() -> Partition {
        Partition { name: "short".into(), default: false, max_minutes: Some(60), cpus: 10, mem_mb: 7492 }
    }

    #[test]
    fn steppers_walk_a_ladder_within_the_partition() {
        let parts = [Partition { default: true, name: "shared".into(), max_minutes: Some(480), ..short() }, short()];
        let mut r = Resources::default();
        apply(&mut r, &Change::Step(Field::Time, true), &parts);
        assert_eq!(r.minutes, 480, "shared allows 8 h at most");
        apply(&mut r, &Change::Step(Field::Cpus, true), &parts);
        assert_eq!(r.cpus, 10, "a node has 10 CPUs");
        apply(&mut r, &Change::Step(Field::Cpus, false), &parts);
        assert_eq!(r.cpus, 8);
        apply(&mut r, &Change::Partition(Some("short".into())), &parts);
        assert_eq!((r.partition.as_deref(), r.minutes, r.mem_gb), (Some("short"), 60, 7));
        apply(&mut r, &Change::Step(Field::Time, false), &parts);
        assert_eq!(r.minutes, 30);
        apply(&mut r, &Change::Preset(0), &parts);
        assert_eq!((r.cpus, r.mem_gb, r.minutes, r.partition.as_deref()), (2, 7, 60, Some("short")));
        let mut r = Resources { minutes: 15, ..Resources::default() };
        apply(&mut r, &Change::Step(Field::Time, false), &[]);
        assert_eq!(r.minutes, 15, "nothing below the ladder");
    }

    #[test]
    fn a_preset_capped_to_the_partition_stays_picked() {
        let parts = [Partition { default: true, name: "shared".into(), max_minutes: Some(480), ..short() }, short()];
        let highlighted = |r: &Resources| (0..3).filter(|&i| is_preset(r, i, &parts)).collect::<Vec<_>>();
        let mut r = Resources::default();
        apply(&mut r, &Change::Preset(0), &parts);
        assert_eq!((r.cpus, r.mem_gb, r.minutes), (2, 7, 120), "Small's 8 GB is more than a node has");
        assert_eq!(highlighted(&r), [0]);
        apply(&mut r, &Change::Preset(1), &parts);
        assert_eq!(highlighted(&r), [1]);
        apply(&mut r, &Change::Partition(Some("short".into())), &parts);
        assert_eq!((r.cpus, r.mem_gb, r.minutes), (8, 7, 60));
        assert_eq!(highlighted(&r), [1], "still Medium, as short allows it");
        apply(&mut r, &Change::Step(Field::Cpus, false), &parts);
        assert!(highlighted(&r).is_empty(), "changed by hand");
        assert!(is_preset(&Resources::preset(2), 2, &[]), "no partitions known: the preset as it is");
    }
}
