"""Lankide's drag-to-install layout; no language-dependent instructions."""

format = "UDZO"
filesystem = "HFS+"
compression_level = 9
files = [(defines["app"], "Lankide.app")]
symlinks = {"Applications": "/Applications"}
background = defines["background"]
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
window_rect = ((100, 100), (640, 400))
default_view = "icon-view"
show_icon_preview = False
arrange_by = None
grid_offset = (0, 0)
grid_spacing = 100
scroll_position = (0, 0)
label_pos = "bottom"
text_size = 14
icon_size = 128
icon_locations = {
    "Lankide.app": (180, 190),
    "Applications": (460, 190),
    ".DS_Store": (1600, 1200),
    ".VolumeIcon.icns": (1760, 1200),
    ".background.tiff": (1920, 1200),
    ".background.png": (2080, 1200),
}
hide = [".background.tiff"]
