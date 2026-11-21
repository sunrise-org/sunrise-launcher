//! Release notes: pulldown-cmark events into gpui elements.
use crate::theme::Theme;
use gpui::{
    div, prelude::*, px, AnyElement, FontStyle, FontWeight, HighlightStyle, SharedString,
    StyledText, UnderlineStyle,
};
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::ops::Range;
use sunrise_launcher::browser::sunrise::sources::{Readme, ReadmeImage};

/// A run of text's styled ranges.
type Spans = Vec<(Range<usize>, Span)>;
/// A table row: each cell's text and its spans.
type Row = Vec<(String, Spans)>;

enum Block {
    Heading(u8, String, Spans),
    Paragraph(String, Spans),
    Item(usize, Option<u64>, String, Spans),
    Code(String),
    Rule,
    Table(Vec<Row>),
    Image(usize),
}

#[derive(Clone)]
enum Span {
    Strong,
    Emphasis,
    Code,
    Link(Option<String>),
}

pub fn render(source: &str, t: &Theme) -> AnyElement {
    render_inner(source, t, None, |_| div().into_any_element())
}

/// A checked README uses the same native Markdown as release notes. Only its pinned,
/// validated links and media are interactive; HTML never becomes a widget.
pub fn render_pinned(
    readme: &Readme,
    t: &Theme,
    image: impl FnMut(&ReadmeImage) -> AnyElement,
) -> AnyElement {
    render_inner(readme.markdown(), t, Some(readme), image)
}

fn render_inner(
    source: &str,
    t: &Theme,
    pinned: Option<&Readme>,
    mut image: impl FnMut(&ReadmeImage) -> AnyElement,
) -> AnyElement {
    let mut blocks = Vec::new();
    let (mut text, mut spans) = (String::new(), Vec::new());
    let mut open: Vec<(Span, usize)> = Vec::new();
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut item: Option<(usize, Option<u64>)> = None;
    let mut heading = None;
    let mut code: Option<String> = None;
    let mut table: Option<Vec<Row>> = None;
    let flush = |blocks: &mut Vec<Block>,
                 text: &mut String,
                 spans: &mut Vec<_>,
                 item: &mut Option<(usize, Option<u64>)>,
                 heading: Option<u8>| {
        if text.trim().is_empty() {
            text.clear();
            spans.clear();
            return;
        }
        let (t, s) = (std::mem::take(text), std::mem::take(spans));
        blocks.push(match (heading, item.take()) {
            (Some(level), _) => Block::Heading(level, t, s),
            (None, Some((depth, n))) => Block::Item(depth, n, t, s),
            _ => Block::Paragraph(t, s),
        });
    };
    let mut in_image = false;
    let mut next_image = 0usize;
    for event in Parser::new_ext(
        source,
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES,
    ) {
        if in_image {
            if event == Event::End(TagEnd::Image) {
                in_image = false;
            }
            continue;
        }
        match event {
            Event::Start(Tag::Image { dest_url, .. }) if pinned.is_some() => {
                if let Some((n, _)) = pinned
                    .into_iter()
                    .flat_map(Readme::images)
                    .enumerate()
                    .skip(next_image)
                    .find(|(_, image)| image.destination() == dest_url.as_ref())
                {
                    flush(&mut blocks, &mut text, &mut spans, &mut item, heading);
                    // Spans from an outer link cannot cross the separately rendered image.
                    for (_, start) in &mut open {
                        *start = 0;
                    }
                    blocks.push(Block::Image(n));
                    next_image = n + 1;
                    in_image = true;
                }
            }
            Event::Start(Tag::Heading { level, .. }) => {
                heading = Some(match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    _ => 3,
                })
            }
            Event::End(TagEnd::Heading(_)) => {
                flush(&mut blocks, &mut text, &mut spans, &mut item, heading);
                heading = None;
            }
            Event::End(TagEnd::Paragraph) => {
                flush(&mut blocks, &mut text, &mut spans, &mut item, None)
            }
            Event::Start(Tag::List(start)) => {
                flush(&mut blocks, &mut text, &mut spans, &mut item, None);
                lists.push(start)
            }
            Event::End(TagEnd::List(_)) => {
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                flush(&mut blocks, &mut text, &mut spans, &mut item, None);
                let depth = lists.len().saturating_sub(1);
                let number = lists.last_mut().and_then(|n| {
                    n.as_mut().map(|n| {
                        *n += 1;
                        *n - 1
                    })
                });
                item = Some((depth, number));
            }
            Event::End(TagEnd::Item) => flush(&mut blocks, &mut text, &mut spans, &mut item, None),
            Event::Start(Tag::CodeBlock(_)) => code = Some(String::new()),
            Event::End(TagEnd::CodeBlock) => {
                blocks.push(Block::Code(code.take().unwrap_or_default()))
            }
            Event::Start(Tag::Strong) => open.push((Span::Strong, text.len())),
            Event::Start(Tag::Emphasis) => open.push((Span::Emphasis, text.len())),
            Event::Start(Tag::Link { dest_url, .. }) => {
                open.push((
                    Span::Link(pinned.and_then(|readme| readme.link(&dest_url))),
                    text.len(),
                ));
            }
            Event::End(TagEnd::Strong | TagEnd::Emphasis | TagEnd::Link) => {
                if let Some((span, start)) = open.pop() {
                    if matches!(&span, Span::Link(Some(_))) && start < text.len() {
                        text.push_str("\u{a0}↗");
                    }
                    spans.push((start..text.len(), span));
                }
            }
            Event::Code(c) => {
                let start = text.len();
                text.push_str(&c);
                spans.push((start..text.len(), Span::Code));
            }
            Event::Text(s) => match &mut code {
                Some(c) => c.push_str(&s),
                None => text.push_str(&s),
            },
            Event::SoftBreak => text.push(' '),
            Event::HardBreak => text.push('\n'),
            Event::Rule => blocks.push(Block::Rule),
            Event::Start(Tag::Table(_)) => {
                flush(&mut blocks, &mut text, &mut spans, &mut item, None);
                table = Some(Vec::new())
            }
            Event::Start(Tag::TableHead | Tag::TableRow) => {
                if let Some(t) = &mut table {
                    t.push(Vec::new())
                }
            }
            Event::End(TagEnd::TableCell) => {
                if let Some(row) = table.as_mut().and_then(|t| t.last_mut()) {
                    row.push((std::mem::take(&mut text), std::mem::take(&mut spans)));
                }
            }
            Event::End(TagEnd::Table) => {
                blocks.push(Block::Table(table.take().unwrap_or_default()))
            }
            _ => {}
        }
    }
    flush(&mut blocks, &mut text, &mut spans, &mut item, None);

    let blocks_drawn = std::cell::Cell::new(0u64);
    let styled = |text: String, spans: Spans| {
        let text_len = text.len();
        let links: Vec<_> = spans
            .iter()
            .filter_map(|(range, span)| match span {
                Span::Link(Some(url)) if !range.is_empty() && range.end <= text_len => {
                    Some((range.clone(), url.clone()))
                }
                _ => None,
            })
            .collect();
        let highlights = spans
            .into_iter()
            .filter(|(range, _)| range.end <= text_len)
            .map(|(range, span)| {
                let style = match span {
                    Span::Strong => HighlightStyle {
                        font_weight: Some(FontWeight::SEMIBOLD),
                        color: Some(t.ink),
                        ..Default::default()
                    },
                    Span::Emphasis => HighlightStyle {
                        font_style: Some(FontStyle::Italic),
                        ..Default::default()
                    },
                    Span::Code => HighlightStyle {
                        background_color: Some(t.plate_soft),
                        color: Some(t.ink),
                        ..Default::default()
                    },
                    Span::Link(url) => HighlightStyle {
                        color: Some(t.ink),
                        underline: url.is_some().then_some(UnderlineStyle {
                            thickness: px(1.),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                };
                (range, style)
            });
        let text = SharedString::from(text);
        // Numbered, so two blocks with the same words are two selections.
        let nth = blocks_drawn.get();
        blocks_drawn.set(nth + 1);
        let id = gpui::ElementId::NamedInteger(text.clone(), nth);
        let text = StyledText::new(text).with_highlights(gpui::combine_highlights(highlights, []));
        if links.is_empty() {
            crate::selectable::Selectable::styled(id, text).into_any_element()
        } else {
            let (ranges, urls): (Vec<_>, Vec<_>) = links.into_iter().unzip();
            gpui::InteractiveText::new(id, text)
                .on_click(ranges, move |i, _, cx| cx.open_url(&urls[i]))
                .into_any_element()
        }
    };
    // The glass look sets notes as the mockup's: 14 px on 1.55 at 75 %, lists with square
    // bullets, 6 px apart.
    let glass = t.glass;
    let mut column = div()
        .flex()
        .flex_col()
        .gap(px(if glass { 6. } else { 8. }))
        .text_size(px(crate::theme::UI.px))
        .line_height(px(if glass { 21.7 } else { 22. }))
        .text_color(t.ink_soft);
    let mut codes = 0usize;
    for block in blocks {
        column = column.child(match block {
            Block::Heading(level, text, spans) => div()
                .pt(px(if level <= 2 { 12. } else { 6. }))
                .text_size(px(if level <= 2 { 16. } else { 14. }))
                .font_weight(if glass {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::MEDIUM
                })
                .text_color(t.ink)
                .child(styled(text, spans))
                .into_any_element(),
            Block::Paragraph(text, spans) => div().child(styled(text, spans)).into_any_element(),
            Block::Item(depth, number, text, spans) => div()
                .flex()
                .gap(px(if glass { 12. } else { 8. }))
                .pl(px(depth as f32 * 16.))
                .child(match number {
                    // The mockup's bullet: a 5 px square, white at 40 %.
                    None if glass => div()
                        .flex_none()
                        .mt(px(8.5))
                        .size(px(5.))
                        .bg(crate::glass_ui::white(0.4)),
                    _ => div().flex_none().w(px(16.)).text_color(t.muted).child(
                        number
                            .map(|n| format!("{n}."))
                            .unwrap_or_else(|| "–".into()),
                    ),
                })
                .child(div().flex_1().min_w_0().child(styled(text, spans)))
                .into_any_element(),
            Block::Code(code) => {
                codes += 1;
                crate::code_box::code_box(format!("md-code-{codes}"), vec![code.into()], None)
                    .rounded(px(if glass {
                        crate::glass_ui::rad(12.)
                    } else {
                        crate::theme::RADIUS_PLATE
                    }))
                    .bg(t.plate_soft)
                    .border_0()
                    .text_size(px(12.))
                    .line_height(px(18.))
                    .text_color(t.ink)
                    .into_any_element()
            }
            Block::Rule => div().h(px(1.)).bg(t.rule).my(px(8.)).into_any_element(),
            Block::Image(n) => image(&pinned.expect("image in a pinned README").images()[n]),
            Block::Table(rows) => {
                let mut grid = div()
                    .flex()
                    .flex_col()
                    .my(px(4.))
                    .border_t_1()
                    .border_color(t.rule);
                for (i, row) in rows.into_iter().enumerate() {
                    let mut line = div()
                        .flex()
                        .gap(px(16.))
                        .py(px(8.))
                        .border_b_1()
                        .border_color(t.rule);
                    let columns = row.len();
                    for (j, (text, spans)) in row.into_iter().enumerate() {
                        // Two-column tables keep a narrow first column; wider ones share the width.
                        let cell = if j == 0 && columns > 1 {
                            div().w(px(180.)).flex_none()
                        } else if j + 1 < columns && columns > 2 {
                            div().w(px(56.)).flex_none()
                        } else {
                            div().flex_1().min_w_0()
                        };
                        line = line.child(
                            cell.when(i == 0, |d| {
                                d.text_color(t.ink)
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_size(px(12.5))
                            })
                            .child(styled(text, spans)),
                        );
                    }
                    grid = grid.child(line);
                }
                grid.into_any_element()
            }
        });
    }
    column.into_any_element()
}
