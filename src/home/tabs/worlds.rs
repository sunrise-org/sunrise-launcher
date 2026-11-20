use super::*;

impl Launcher {
    pub(super) fn home_worlds(
        &mut self,
        instance: &instances::Instance,
        compact: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Content {
        self.load_worlds(instance.home.clone(), cx);
        let count = self
            .worlds
            .fresh
            .then_some(self.worlds.list.len())
            .or_else(|| (!self.worlds.list.is_empty()).then_some(self.worlds.list.len()));
        let view = &self.ui.home.tabs.views[Tab::Worlds.index()];
        let tiles = self.settings.world_layout == backend::WorldLayout::Tiles;
        let across = if tiles {
            view.near.width().map_or(1, |wide| {
                ((wide + TILE_GAP) / (WORLD_TILE + TILE_GAP)).max(1.) as usize
            })
        } else {
            1
        };
        let (height, gap) = if tiles {
            (TILE_TALL, TILE_GAP)
        } else {
            (ROW, 0.)
        };
        let (built, place) =
            view.near
                .list(&view.scroll, self.worlds.list.len(), across, height, gap);
        let items = if count == Some(0) {
            empty(
                "earth",
                tr("No worlds here yet"),
                tr("Press Play and make a world in the game. It shows up here, ready to continue."),
                None,
            )
        } else if self.worlds.list.is_empty() {
            div()
        } else {
            let worlds = self.worlds.list[built].to_vec();
            let mut rows = div().flex().gap(px(gap));
            if tiles {
                rows = rows.flex_wrap();
                for world in &worlds {
                    rows = rows.child(self.world_tile(world, window, cx));
                }
            } else {
                rows = rows.flex_col();
                for world in &worlds {
                    rows = rows.child(self.world_row(world, compact, cx));
                }
            }
            place(rows)
        };
        Content {
            count,
            manage: None,
            extra: None,
            items,
        }
    }
}
