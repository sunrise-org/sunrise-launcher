//! What goes into a mod's or pack's card: its picture and the muted line under its name.
use super::*;
use crate::mods::selection::item::CardFacts;
use crate::mods::HomeModFacts;

impl Launcher {
    /// The picture of `item`'s card: a registry mod's icon, a local package's own (or its
    /// source's), a pack's artwork; a letter tile until the picture is there.
    pub(super) fn home_item_picture(&mut self, item: &Item, cx: &mut Context<Self>) -> AnyElement {
        let picture = match &item.picture {
            Picture::Project(project) => {
                let icon = sunrise::v2::shown_index()
                    .and_then(|catalog| Some(catalog.index.project(project)?.icon.clone()));
                let image = icon.and_then(|icon| {
                    self.v2_mod_image(project, &icon, sunrise_launcher::browser::images::ROW)
                });
                tile(image, &item.id, &item.name, PICTURE)
            }
            Picture::Local { package, source } => {
                let listing = if let Some(origin) = source {
                    self.read_nightly_catalogue(false);
                    self.source_listing(origin)
                } else {
                    None
                };
                if let Some(source) = listing {
                    self.source_tile(
                        source,
                        None,
                        self.nightly_catalogue_current(),
                        package.clone(),
                        PICTURE,
                        cx,
                    )
                } else {
                    tile(package.clone(), &item.id, &item.name, PICTURE)
                }
            }
            Picture::Pack(art) => tile(art.show(cx), &item.id, &item.name, PICTURE),
        };
        picture.into_any_element()
    }

    /// The muted line under a mod's name: what kind of mod it is, its version, where it came
    /// from, and what Installed says about it (needs a build, update failed…). `label` is what
    /// Installed says of a mod that is on; one that is off has only its version and source.
    pub(super) fn home_mod_detail(
        item: &Item,
        label: Option<&HomeModFacts>,
        facts: Option<&CardFacts>,
    ) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(kind) = label
            .map(|label| label.kind)
            .filter(|kind| !kind.is_empty())
        {
            parts.push(kind.to_owned());
        }
        let version = facts
            .map(|facts| facts.version.clone())
            .or_else(|| item.mod_version.clone())
            .filter(|version| !version.is_empty());
        parts.extend(version);
        let badge = label.and_then(|label| label.badge).or(item.manual_badge);
        parts.push(match badge {
            Some(badge) => badge.label().to_owned(),
            None => tr("Sunrise registry").to_owned(),
        });
        parts.extend(label.and_then(|label| label.status.clone()));
        parts.join(" · ")
    }

    /// The muted line under a pack's name: whether it is on, and for a resource pack its place
    /// in the stack. Shader packs have no stack: one is on, or none.
    pub(super) fn home_pack_detail(
        collection: Collection,
        item: &Item,
        priority: &mut usize,
    ) -> String {
        if collection == Collection::Shader {
            return if item.on { tr("On") } else { tr("Off") }.to_owned();
        }
        if item.on {
            *priority += 1;
            trf(
                "Enabled · {priority}",
                &[("priority", &priority.to_string())],
            )
        } else {
            tr("Off").to_owned()
        }
    }
}
