use super::*;

impl Cx<'_> {
    /// With images on: room for the picture when it is known, else a line of text saying why
    /// not; then the caption. Without: `[image alt · W×H]`.
    pub(crate) fn image(&mut self, v: &pb::ImageComponent, w: usize) -> Lines {
        let muted = self.p.muted_style();
        let name = v
            .alt
            .as_deref()
            .map(one_line)
            .filter(|a| !a.is_empty())
            .unwrap_or_else(|| one_line(&v.src));
        let mut out = match self.o.images.as_ref() {
            None => {
                let mut s = format!("[image {name}");
                if let (Some(iw), Some(ih)) = (v.width, v.height) {
                    s.push_str(&format!(" · {iw}×{ih}"));
                }
                s.push(']');
                wrap(&[Span::styled(s, muted)], w)
            }
            Some(images) => match images.status.get(&v.src) {
                Some(ImageStatus::Ready { width, height }) => {
                    let name = v
                        .alt
                        .as_deref()
                        .map(one_line)
                        .filter(|a| !a.is_empty())
                        .unwrap_or_else(|| file_name(&v.src));
                    self.room(images, &v.src, (*width, *height), &name, w)
                }
                Some(ImageStatus::Failed(why)) => wrap(
                    &[Span::styled(
                        format!("[image {name} · {}]", one_line(why)),
                        muted,
                    )],
                    w,
                ),
                _ => wrap(
                    &[Span::styled(
                        format!("[image {name}] loading image…"),
                        muted,
                    )],
                    w,
                ),
            },
        };
        if let Some(cap) = v.caption.as_deref().map(clean).filter(|c| !c.is_empty()) {
            out.extend(wrap(&[Span::raw(cap.into_owned())], w));
        }
        out
    }

    /// The marked first line, the empty lines under it, then `name · W×H`.
    pub(super) fn room(
        &mut self,
        images: &ImageOptions,
        src: &str,
        size: (u32, u32),
        name: &str,
        w: usize,
    ) -> Lines {
        let max_cols = u16::try_from(w).unwrap_or(u16::MAX);
        let (cols, rows) = filled_cells(size.0, size.1, images.cell, max_cols, images.max_rows);
        let mut out = self.reserve_cells(src, cols, rows);
        let caption = format!("{name} · {}×{}", size.0, size.1);
        out.extend(wrap(&[Span::styled(caption, self.p.muted_style())], w));
        out
    }

    /// How many of `children`, from the first, are pictures ready to draw: a run of them shares
    /// the width.
    pub(super) fn ready_pictures(&self, children: &[pb::Component]) -> usize {
        let Some(images) = self.o.images.as_ref() else {
            return 0;
        };
        children
            .iter()
            .take_while(|c| match c {
                pb::Component::ImageComponent(v) => {
                    matches!(images.status.get(&v.src), Some(ImageStatus::Ready { .. }))
                }
                _ => false,
            })
            .count()
    }

    /// Pictures side by side, up to three a row, each row as tall as its least tall picture
    /// would be in its share of the width (so they line up), each with its name under it.
    pub(super) fn pictures(&mut self, pictures: &[&pb::ImageComponent], w: usize) -> Lines {
        let Some(images) = self.o.images.as_ref() else {
            return Vec::new();
        };
        let size = |v: &pb::ImageComponent| match images.status.get(&v.src) {
            Some(ImageStatus::Ready { width, height }) => (*width, *height),
            _ => (1, 1),
        };
        let per_row = (w + GAP) / (MIN_PICTURE + GAP);
        let per_row = per_row.clamp(1, PICTURES_PER_ROW);
        let mut out: Lines = Vec::new();
        for row in pictures.chunks(per_row) {
            if !out.is_empty() {
                out.push(Line::default());
            }
            let share = (w.saturating_sub(GAP * (row.len() - 1)) / row.len()).max(1);
            let share16 = u16::try_from(share).unwrap_or(u16::MAX);
            let tall = row
                .iter()
                .map(|v| {
                    let (iw, ih) = size(v);
                    filled_cells(iw, ih, images.cell, share16, images.max_rows).1
                })
                .min()
                .unwrap_or(1);
            let parts: Vec<(Lines, usize)> = row
                .iter()
                .map(|v| {
                    let (iw, ih) = size(v);
                    let (cols, rows) = filled_cells(iw, ih, images.cell, share16, tall);
                    let mut lines = self.reserve_cells(&v.src, cols, rows);
                    lines.resize(usize::from(tall), Line::default());
                    let name = v
                        .alt
                        .as_deref()
                        .map(one_line)
                        .filter(|a| !a.is_empty())
                        .unwrap_or_else(|| file_name(&v.src));
                    let caption = truncate(&format!("{name} · {iw}×{ih}"), share);
                    lines.push(sline(caption, self.p.muted_style()));
                    (lines, share)
                })
                .collect();
            out.extend(crate::neoui::columns(parts, &vec![GAP; row.len()], "start"));
        }
        out
    }

    /// Room for a picture of `size` pixels in at most `max_cols` x `max_rows` cells: the marked
    /// first line and the empty lines under it, and the cells taken. Measuring (a quiet draw)
    /// reserves nothing, since its lines are thrown away.
    pub(crate) fn reserve(
        &mut self,
        images: &ImageOptions,
        src: &str,
        (iw, ih): (u32, u32),
        max_cols: u16,
        max_rows: u16,
    ) -> (Lines, (u16, u16)) {
        let (cols, rows) = image_cells(iw, ih, images.cell, max_cols, max_rows);
        (self.reserve_cells(src, cols, rows), (cols, rows))
    }

    /// Room for a picture of `cols` x `rows` cells: the marked first line and the empty lines
    /// under it (what is put before the mark moves the picture right).
    pub(crate) fn reserve_cells(&mut self, src: &str, cols: u16, rows: u16) -> Lines {
        if !self.ui.drawing_to_measure() {
            self.rooms.push((cols, rows, src.to_string()));
        }
        let mut out = vec![Line::from(Span::raw(SLOT_MARK))];
        out.resize(usize::from(rows), Line::default());
        out
    }

    pub(super) fn file(&self, v: &pb::FileComponent, w: usize) -> Lines {
        let path = one_line(&v.path);
        let mut spans = vec![Span::raw("📄 ")];
        match v
            .name
            .as_deref()
            .map(one_line)
            .filter(|n| !n.is_empty() && *n != path)
        {
            Some(name) => {
                spans.push(Span::raw(name));
                spans.push(Span::styled(format!(" ({path})"), self.p.muted_style()));
            }
            None => spans.push(Span::raw(path)),
        }
        wrap(&spans, w)
    }
}
