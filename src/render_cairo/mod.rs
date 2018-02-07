// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Cairo backend implementation.

use std::f64;

// external
use cairo::{
    self,
    MatrixTrait,
};

// self
use tree::{
    self,
    NodeExt,
};
use math::*;
use traits::{
    ConvTransform,
    TransformFromBBox,
};
use {
    ErrorKind,
    Options,
    Result,
};
use render_utils;
use self::ext::*;


mod clippath;
mod ext;
mod fill;
mod gradient;
mod image;
mod path;
mod pattern;
mod stroke;
mod text;


impl ConvTransform<cairo::Matrix> for tree::Transform {
    fn to_native(&self) -> cairo::Matrix {
        cairo::Matrix::new(self.a, self.b, self.c, self.d, self.e, self.f)
    }

    fn from_native(ts: &cairo::Matrix) -> Self {
        Self::new(ts.xx, ts.yx, ts.xy, ts.yy, ts.x0, ts.y0)
    }
}

impl TransformFromBBox for cairo::Matrix {
    fn from_bbox(bbox: Rect) -> Self {
        Self::new(bbox.width(), 0.0, 0.0, bbox.height(), bbox.x(), bbox.y())
    }
}


/// Renders SVG to image.
pub fn render_to_image(
    rtree: &tree::RenderTree,
    opt: &Options,
) -> Result<cairo::ImageSurface> {
    let img_size = render_utils::fit_to(rtree.svg_node().size, opt.fit_to);

    debug_assert!(!img_size.is_empty_or_negative());

    let surface = cairo::ImageSurface::create(
        cairo::Format::ARgb32,
        img_size.width as i32,
        img_size.height as i32
    );

    let surface = match surface {
        Ok(v) => v,
        Err(_) => {
            return Err(ErrorKind::NoCanvas.into());
        }
    };

    let img_view = Rect::new(Point::new(0.0, 0.0), img_size);
    let cr = cairo::Context::new(&surface);

    // Fill background.
    if let Some(color) = opt.background {
        cr.set_source_color(&color, 1.0);
        cr.paint();
    }

    render_to_canvas(&cr, img_view, rtree);

    Ok(surface)
}

/// Renders SVG to canvas.
pub fn render_to_canvas(
    cr: &cairo::Context,
    img_view: Rect,
    rtree: &tree::RenderTree,
) {
    // Apply viewBox.
    let ts = {
        let vbox = rtree.svg_node().view_box;
        let (dx, dy, sx, sy) = render_utils::view_box_transform(vbox, img_view);
        cairo::Matrix::new(sx, 0.0, 0.0, sy, dx, dy)
    };
    cr.transform(ts);

    render_group(rtree, rtree.root(), &cr, &cr.get_matrix(), img_view.size);
}

fn render_group(
    rtree: &tree::RenderTree,
    node: tree::NodeRef,
    cr: &cairo::Context,
    matrix: &cairo::Matrix,
    img_size: Size,
) -> Rect {
    let mut g_bbox = Rect::from_xywh(f64::MAX, f64::MAX, 0.0, 0.0);

    for node in node.children() {
        cr.transform(node.transform().to_native());

        let bbox = match *node.value() {
            tree::NodeKind::Path(ref path) => {
                Some(path::draw(rtree, path, cr))
            }
            tree::NodeKind::Text(_) => {
                Some(text::draw(rtree, node, cr))
            }
            tree::NodeKind::Image(ref img) => {
                Some(image::draw(img, cr))
            }
            tree::NodeKind::Group(ref g) => {
                render_group_impl(rtree, node, g, cr, img_size)
            }
            _ => None,
        };

        if let Some(bbox) = bbox {
            g_bbox.expand_from_rect(bbox);
        }

        cr.set_matrix(*matrix);
    }

    g_bbox
}

fn render_group_impl(
    rtree: &tree::RenderTree,
    node: tree::NodeRef,
    g: &tree::Group,
    cr: &cairo::Context,
    img_size: Size,
) -> Option<Rect> {
    let sub_surface = cairo::ImageSurface::create(
        cairo::Format::ARgb32,
        img_size.width as i32,
        img_size.height as i32
    );

    let sub_surface = match sub_surface {
        Ok(surf) => surf,
        Err(_) => {
            warn!("Subsurface creation failed.");
            return None;
        }
    };

    let sub_cr = cairo::Context::new(&sub_surface);
    sub_cr.set_matrix(cr.get_matrix());

    let bbox = render_group(rtree, node, &sub_cr, &cr.get_matrix(), img_size);

    if let Some(idx) = g.clip_path {
        let clip_node = rtree.defs_at(idx);
        if let tree::NodeKind::ClipPath(ref cp) = *clip_node.value() {
            clippath::apply(rtree, clip_node, cp, &sub_cr, bbox, img_size);
        }
    }

    let curr_matrix = cr.get_matrix();
    cr.set_matrix(cairo::Matrix::identity());

    cr.set_source_surface(&sub_surface, 0.0, 0.0);

    if let Some(opacity) = g.opacity {
        cr.paint_with_alpha(opacity);
    } else {
        cr.paint();
    }

    cr.set_matrix(curr_matrix);

    Some(bbox)
}
