use std::sync::Arc;

use gpui::*;

/// A square, embedded SVG icon that preserves its original colors.
/// Use theme-colored `Icon`/`svg()` for monochrome symbols instead.
#[derive(Clone)]
pub struct DesktopColorIcon {
    path: SharedString,
    size: Pixels,
}

impl DesktopColorIcon {
    pub fn new(path: impl Into<SharedString>) -> Self {
        Self {
            path: path.into(),
            size: px(16.),
        }
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }
}

impl IntoElement for DesktopColorIcon {
    type Element = Img;

    fn into_element(self) -> Self::Element {
        let logical_size = self.size;
        img(move |window: &mut Window, cx: &mut App| {
            let edge = (f32::from(logical_size) * window.scale_factor()).round() as i32;
            if edge <= 0 {
                return None;
            }
            // The cache key follows both icon size and the current display's DPI.
            // Never use img(path)'s intrinsic-size SVG raster and scale it later.
            window.use_asset::<AssetLogger<ColorIconRaster>>(
                &(self.path.clone(), DevicePixels(edge)),
                cx,
            )
        })
        .size(logical_size)
        .flex_none()
        .object_fit(ObjectFit::Contain)
    }
}

struct ColorIconDocument;

impl Asset for ColorIconDocument {
    type Source = SharedString;
    type Output = Result<Arc<ParsedSvg>, ImageCacheError>;

    fn load(
        path: Self::Source,
        cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        let assets = cx.asset_source().clone();
        let renderer = cx.svg_renderer();
        async move {
            let bytes = assets.load(&path)?.ok_or_else(|| {
                ImageCacheError::Asset(format!("Missing SVG icon: {path}").into())
            })?;
            renderer.parse_svg(&bytes).map(Arc::new).map_err(Into::into)
        }
    }
}

struct ColorIconRaster;

impl Asset for ColorIconRaster {
    type Source = (SharedString, DevicePixels);
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        let (document, _) = cx.fetch_asset::<ColorIconDocument>(&source.0);
        let renderer = cx.svg_renderer();
        async move {
            let document = document.await?;
            renderer
                .render_parsed(&document, SvgSize::Size(size(source.1, source.1)))
                .map_err(Into::into)
        }
    }
}
