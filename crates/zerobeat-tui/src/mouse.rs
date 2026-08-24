use ratatui::layout::Rect;
use zerobeat_core::Route;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseTarget {
    Navigation(Route),
    SearchInput,
    ContentTrack(usize),
    QueueTrack(usize),
    Progress,
    Shuffle,
    Previous,
    PlayPause,
    Next,
    Repeat,
    Like,
    Lyrics,
    Mute,
    Queue,
    Player,
}

#[derive(Clone, Copy, Debug)]
struct HitRegion {
    target: MouseTarget,
    area: Rect,
}

#[derive(Clone, Debug, Default)]
pub struct HitMap {
    regions: Vec<HitRegion>,
}

impl HitMap {
    pub fn region(&self, target: MouseTarget) -> Option<Rect> {
        self.regions
            .iter()
            .find(|region| region.target == target)
            .map(|region| region.area)
    }

    pub(crate) fn add(&mut self, target: MouseTarget, area: Rect) {
        if area.width > 0 && area.height > 0 {
            self.regions.push(HitRegion { target, area });
        }
    }

    pub(crate) fn target_at(&self, column: u16, row: u16) -> Option<MouseTarget> {
        self.regions
            .iter()
            .rev()
            .find(|region| contains(region.area, column, row))
            .map(|region| region.target)
    }

    pub(crate) fn contains(&self, target: MouseTarget, column: u16, row: u16) -> bool {
        self.region(target)
            .is_some_and(|area| contains(area, column, row))
    }
}

fn contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x
        && column < area.x.saturating_add(area.width)
        && row >= area.y
        && row < area.y.saturating_add(area.height)
}
