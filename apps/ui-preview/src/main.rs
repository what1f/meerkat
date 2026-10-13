use desktop_ui::{
    ContentPane, DesktopAvatar, DesktopAvatarGroup, DesktopButton, DesktopDialogExt,
    DesktopDialogFooter, DesktopDialogHeader, DesktopFormField, DesktopInput, DesktopLanguage,
    DesktopSelect, DesktopShell, DesktopTextarea, DesktopTheme, Pane, PaneHeader, RADIUS_CARD,
    RADIUS_ROW, Sidebar, SidebarItem, SidebarSectionLabel, ThemePreference, apply_component_theme,
    configure_window_material, desktop_choice, desktop_dialog_button, desktop_dialog_cancel,
    desktop_settings_group, desktop_settings_row, desktop_settings_section_title,
    observe_window_frame,
};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Icon, IconName, IndexPath, Root, Sizable as _, Theme as ComponentTheme,
    ThemeMode as ComponentThemeMode, WindowExt as _,
    input::{InputState, TextareaState},
    list::ListItem,
    menu::{DropdownMenu as _, PopupMenuItem},
    select::{SelectEvent, SelectState},
    tree::{TreeItem, TreeState, tree},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum PreviewPage {
    Layout,
    Controls,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PreviewLayout {
    TwoPane,
    ThreePane,
}

struct PreviewApp {
    page: PreviewPage,
    layout: PreviewLayout,
    dark: bool,
    theme_preference: ThemePreference,
    language: DesktopLanguage,
    focus_handle: FocusHandle,
    input: Entity<InputState>,
    textarea: Entity<TextareaState>,
    select: Entity<SelectState<Vec<String>>>,
    language_select: Entity<SelectState<Vec<String>>>,
    dialog_input: Entity<InputState>,
    tree: Entity<TreeState>,
}

impl PreviewApp {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("输入名称，例如 Desktop UI"));
        let textarea = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(4)
                .placeholder("补充说明或备注…")
        });
        let select = cx.new(|cx| {
            SelectState::new(
                vec!["设计".into(), "开发".into(), "文档".into()],
                Some(IndexPath::default()),
                window,
                cx,
            )
        });
        let dialog_input = cx.new(|cx| InputState::new(window, cx).placeholder("请输入工作区名称"));
        let language_select = cx.new(|cx| {
            SelectState::new(
                vec!["简体中文".into(), "English".into()],
                Some(IndexPath::default()),
                window,
                cx,
            )
        });
        let tree = cx.new(|cx| {
            TreeState::new(cx).items(vec![
                TreeItem::new("src", "src")
                    .expanded(true)
                    .child(TreeItem::new("src/components", "components").children([
                        TreeItem::new("src/components/button.rs", "button.rs"),
                        TreeItem::new("src/components/input.rs", "input.rs"),
                    ]))
                    .child(TreeItem::new("src/lib.rs", "lib.rs")),
                TreeItem::new("Cargo.toml", "Cargo.toml"),
                TreeItem::new("README.md", "README.md"),
            ])
        });

        cx.subscribe_in(
            &language_select,
            window,
            |this, _, event: &SelectEvent<Vec<String>>, window, cx| {
                let SelectEvent::Confirm(Some(value)) = event else {
                    return;
                };
                this.language = if *value == "English" {
                    DesktopLanguage::English
                } else {
                    DesktopLanguage::SimplifiedChinese
                };
                let placeholder =
                    this.tr("输入名称，例如 Desktop UI", "Enter a name, e.g. Desktop UI");
                this.input.update(cx, |input, cx| {
                    input.set_placeholder(placeholder, window, cx)
                });
                let textarea_placeholder = this.tr("补充说明或备注…", "Add a description or note…");
                this.textarea.update(cx, |input, cx| {
                    input.set_placeholder(textarea_placeholder, window, cx)
                });
                let dialog_placeholder = this.tr("请输入工作区名称", "Enter a workspace name");
                this.dialog_input.update(cx, |input, cx| {
                    input.set_placeholder(dialog_placeholder, window, cx)
                });
                let select_items = if this.language == DesktopLanguage::English {
                    vec![
                        "Design".into(),
                        "Development".into(),
                        "Documentation".into(),
                    ]
                } else {
                    vec!["设计".into(), "开发".into(), "文档".into()]
                };
                this.select.update(cx, |select, cx| {
                    select.set_items(select_items, window, cx);
                    select.set_selected_index(Some(IndexPath::default()), window, cx);
                });
                this.focus_handle.focus(window, cx);
                cx.notify();
            },
        )
        .detach();

        Self {
            page: PreviewPage::Layout,
            layout: PreviewLayout::ThreePane,
            dark: false,
            theme_preference: ThemePreference::Light,
            language: DesktopLanguage::SimplifiedChinese,
            focus_handle: cx.focus_handle(),
            input,
            textarea,
            select,
            language_select,
            dialog_input,
            tree,
        }
    }

    fn tr(&self, zh: &'static str, en: &'static str) -> &'static str {
        self.language.text(zh, en)
    }

    fn theme(&self) -> DesktopTheme {
        if self.dark {
            DesktopTheme::dark()
        } else {
            DesktopTheme::light()
        }
    }

    fn brand(&self) -> impl IntoElement {
        let theme = self.theme();
        div()
            .h(px(42.))
            .px(px(16.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(9.))
            .child(
                div()
                    .size(px(22.))
                    .rounded(px(7.5))
                    .bg(theme.text)
                    .text_color(theme.content)
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("UI"),
            )
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Desktop UI"))
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.theme();
        Sidebar::new(theme)
            .header(self.brand())
            .child(SidebarSectionLabel::new(self.tr("预览", "Preview"), theme))
            .child(
                div()
                    .id("preview-two-pane")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.page = PreviewPage::Layout;
                            this.layout = PreviewLayout::TwoPane;
                            this.focus_handle.focus(window, cx);
                            cx.notify();
                        }),
                    )
                    .child(
                        SidebarItem::new(
                            "preview-two-pane-item",
                            self.tr("两栏布局", "Two-pane layout"),
                            theme,
                        )
                        .leading_element(
                            Icon::new(IconName::PanelLeft).text_color(theme.text_secondary),
                        )
                        .selected(
                            self.page == PreviewPage::Layout
                                && self.layout == PreviewLayout::TwoPane,
                        ),
                    ),
            )
            .child(
                div()
                    .id("preview-three-pane")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.page = PreviewPage::Layout;
                            this.layout = PreviewLayout::ThreePane;
                            this.focus_handle.focus(window, cx);
                            cx.notify();
                        }),
                    )
                    .child(
                        SidebarItem::new(
                            "preview-three-pane-item",
                            self.tr("三栏布局", "Three-pane layout"),
                            theme,
                        )
                        .leading_element(
                            Icon::new(IconName::LayoutDashboard).text_color(theme.text_secondary),
                        )
                        .selected(
                            self.page == PreviewPage::Layout
                                && self.layout == PreviewLayout::ThreePane,
                        ),
                    ),
            )
            .child(SidebarSectionLabel::new(
                self.tr("基础组件", "Components"),
                theme,
            ))
            .child(
                div()
                    .id("preview-controls")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.page = PreviewPage::Controls;
                            this.focus_handle.focus(window, cx);
                            cx.notify();
                        }),
                    )
                    .child(
                        SidebarItem::new(
                            "preview-controls-item",
                            self.tr("常用组件", "Common components"),
                            theme,
                        )
                        .leading_element(
                            Icon::new(IconName::Palette).text_color(theme.text_secondary),
                        )
                        .selected(self.page == PreviewPage::Controls),
                    ),
            )
            .footer(
                div()
                    .p(px(8.))
                    .border_t_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .id("preview-settings")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, window, cx| {
                                    this.page = PreviewPage::Settings;
                                    this.focus_handle.focus(window, cx);
                                    cx.notify();
                                }),
                            )
                            .child(
                                SidebarItem::new(
                                    "preview-settings-item",
                                    self.tr("设置", "Settings"),
                                    theme,
                                )
                                .leading_element(
                                    Icon::new(IconName::Settings).text_color(theme.text_secondary),
                                )
                                .selected(self.page == PreviewPage::Settings),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn list_pane(&self) -> impl IntoElement {
        let theme = self.theme();
        Pane::new(
            theme,
            div()
                .p(px(8.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    SidebarItem::new("layout-sidebar", self.tr("侧边栏", "Sidebar"), theme)
                        .leading("S")
                        .selected(true),
                )
                .child(
                    SidebarItem::new("layout-pane", self.tr("左侧面板", "Pane"), theme)
                        .leading("P"),
                )
                .child(
                    SidebarItem::new("layout-content", self.tr("内容区", "Content pane"), theme)
                        .leading("C"),
                ),
        )
        .header(
            PaneHeader::new(self.tr("布局", "Layout"), theme).trailing(
                DesktopButton::icon("layout-add")
                    .icon(IconName::Plus)
                    .tooltip("添加"),
            ),
        )
    }

    fn layout_content(&self) -> impl IntoElement {
        let theme = self.theme();
        let columns = if self.layout == PreviewLayout::ThreePane {
            self.tr("侧边栏 · 面板 · 内容区", "Sidebar · Pane · Content")
        } else {
            self.tr("侧边栏 · 内容区", "Sidebar · Content")
        };
        ContentPane::new(
            theme,
            div()
                .max_w(px(760.))
                .p(px(32.))
                .child(page_title(
                    self.tr("布局预览", "Layout preview"),
                    columns,
                    theme,
                ))
                .child(section_title(self.tr("表面色彩", "Surface tokens")))
                .child(div().mt(px(10.)).flex().gap(px(10.)).children([
                    swatch(self.tr("窗口", "Window"), theme.window, theme),
                    swatch(self.tr("侧边栏", "Sidebar"), theme.sidebar, theme),
                    swatch(self.tr("面板", "Pane"), theme.pane, theme),
                    swatch(self.tr("浮层", "Raised"), theme.raised, theme),
                ])),
        )
        .header(PaneHeader::new(
            self.tr("组件预览", "Component preview"),
            theme,
        ))
    }

    fn controls_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme();
        let dialog_input = self.dialog_input.clone();
        let tree_state = self.tree.clone();
        let dialog_title = self.tr("新建工作区", "New workspace");
        let dialog_field = self.tr("工作区名称", "Workspace name");
        let dialog_cancel = self.tr("取消", "Cancel");
        let dialog_create = self.tr("创建", "Create");

        ContentPane::new(
            theme,
            div()
                .w_full()
                .max_w(px(820.))
                .p(px(32.))
                .pb(px(48.))
                .child(page_title(
                    self.tr("常用组件", "Common components"),
                    self.tr(
                        "这里的控件都可以直接点击、输入和展开。",
                        "Every control here is interactive.",
                    ),
                    theme,
                ))
                .child(
                    div()
                        .mt(px(24.))
                        .grid()
                        .grid_cols(2)
                        .gap(px(32.))
                        .child(
                            component_section(self.tr("表单控件", "Form controls"))
                                .child(DesktopFormField::new(
                                    theme,
                                    self.tr("单行输入框", "Single-line input"),
                                    DesktopInput::new(&self.input),
                                ))
                                .child(DesktopFormField::new(
                                    theme,
                                    "Textarea",
                                    DesktopTextarea::new(&self.textarea).h(px(104.)),
                                ))
                                .child(DesktopFormField::new(
                                    theme,
                                    self.tr("下拉选择框", "Select"),
                                    DesktopSelect::new(&self.select).w_full(),
                                )),
                        )
                        .child(
                            component_section(
                                self.tr("按钮、菜单与弹窗", "Buttons, menus and dialogs"),
                            )
                            .child(field_label(self.tr("纯 Icon 按钮", "Icon buttons"), theme))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(
                                        DesktopButton::icon("icon-add")
                                            .icon(IconName::Plus)
                                            .tooltip("新建"),
                                    )
                                    .child(
                                        DesktopButton::icon("icon-settings")
                                            .icon(IconName::Settings)
                                            .tooltip("设置"),
                                    )
                                    .child(
                                        DesktopButton::icon("icon-more")
                                            .icon(IconName::Ellipsis)
                                            .tooltip("更多")
                                            .dropdown_menu(|menu, _, _| {
                                                menu.item(PopupMenuItem::new("重命名"))
                                                    .item(PopupMenuItem::new("复制"))
                                                    .separator()
                                                    .item(PopupMenuItem::new("删除"))
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .mt(px(14.))
                                    .child(field_label(self.tr("弹窗表单", "Dialog form"), theme)),
                            )
                            .child(
                                DesktopButton::primary(
                                    "open-form-dialog-hitbox",
                                    self.tr("打开弹窗", "Open dialog"),
                                )
                                .on_click(cx.listener(
                                    move |_, _, window, cx| {
                                        let input = dialog_input.clone();
                                        cx.defer_in(window, move |_, window, cx| {
                                            window.open_dialog(cx, move |dialog, window, _| {
                                                let dialog_height = px(250.);
                                                dialog
                                                    .desktop()
                                                    .w(px(520.))
                                                    .h(dialog_height)
                                                    .margin_top(
                                                        (window.viewport_size().height
                                                            - dialog_height)
                                                            / 2.,
                                                    )
                                                    .child(
                                                        div()
                                                            .flex()
                                                            .flex_col()
                                                            .h_full()
                                                            .child(DesktopDialogHeader::new(
                                                                dialog_title,
                                                            ))
                                                            .child(
                                                                div().px(px(22.)).pt(px(4.)).child(
                                                                    DesktopFormField::new(
                                                                        theme,
                                                                        dialog_field,
                                                                        DesktopInput::new(&input),
                                                                    ),
                                                                ),
                                                            )
                                                            .child(
                                                                DesktopDialogFooter::new()
                                                                    .child(desktop_dialog_cancel(
                                                                        "dialog-cancel",
                                                                        dialog_cancel,
                                                                    ))
                                                                    .child(desktop_dialog_button(
                                                                        "dialog-save",
                                                                        DesktopButton::primary(
                                                                            "dialog-save-button",
                                                                            dialog_create,
                                                                        )
                                                                        .on_click(
                                                                            |_, window, cx| {
                                                                                window.close_dialog(
                                                                                    cx,
                                                                                )
                                                                            },
                                                                        ),
                                                                    )),
                                                            ),
                                                    )
                                            });
                                        });
                                    },
                                )),
                            ),
                        )
                        .child(
                            component_section(self.tr("文件树", "File tree"))
                                .child(field_label(
                                    self.tr(
                                        "点击文件夹展开，右键任意项目打开菜单。",
                                        "Expand folders or right-click an item for its menu.",
                                    ),
                                    theme,
                                ))
                                .child(
                                    div()
                                        .h(px(210.))
                                        .rounded(px(20.))
                                        .border_1()
                                        .border_color(theme.border)
                                        .overflow_hidden()
                                        .child(
                                            tree(&tree_state, move |ix, entry, selected, _, cx| {
                                                let item = entry.item();
                                                let icon = if !entry.is_folder() {
                                                    IconName::File
                                                } else if entry.is_expanded() {
                                                    IconName::FolderOpen
                                                } else {
                                                    IconName::Folder
                                                };
                                                ListItem::new(ix)
                                                    .selected(selected)
                                                    .rounded(px(RADIUS_ROW))
                                                    .px(px(10.))
                                                    .pl(px(16.) * entry.depth() + px(10.))
                                                    .child(
                                                        div()
                                                            .flex()
                                                            .items_center()
                                                            .gap(px(7.))
                                                            .child(
                                                                Icon::new(icon).small().text_color(
                                                                    cx.theme().muted_foreground,
                                                                ),
                                                            )
                                                            .child(item.label.clone()),
                                                    )
                                            })
                                            .context_menu(|_, _, menu, _, _| {
                                                menu.item(PopupMenuItem::new("打开"))
                                                    .item(PopupMenuItem::new("重命名"))
                                                    .separator()
                                                    .item(PopupMenuItem::new("删除"))
                                            })
                                            .p(px(4.)),
                                        ),
                                ),
                        )
                        .child(
                            component_section(self.tr("头像", "Avatars"))
                                .child(field_label(self.tr("单头像", "Avatar"), theme))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(10.))
                                        .child(DesktopAvatar::new("Forest He", theme).size(px(40.)))
                                        .child(
                                            div()
                                                .text_size(px(13.))
                                                .text_color(theme.text_secondary)
                                                .child("Forest He"),
                                        ),
                                )
                                .child(
                                    div().mt(px(10.)).child(field_label(
                                        self.tr("头像组", "Avatar group"),
                                        theme,
                                    )),
                                )
                                .child(
                                    DesktopAvatarGroup::new(theme)
                                        .size(px(32.))
                                        .overlap(px(6.))
                                        .limit(4)
                                        .children([
                                            DesktopAvatar::new("Alice Chen", theme),
                                            DesktopAvatar::new("Bob Li", theme),
                                            DesktopAvatar::new("Carol Wang", theme),
                                            DesktopAvatar::new("David Zhou", theme),
                                            DesktopAvatar::new("Eve Lin", theme),
                                        ]),
                                ),
                        ),
                ),
        )
        .id("controls-content-scroll")
        .header(
            PaneHeader::new(self.tr("组件预览", "Component preview"), theme).trailing(
                DesktopButton::icon("header-more")
                    .icon(IconName::Ellipsis)
                    .dropdown_menu(|menu, _, _| {
                        menu.item(PopupMenuItem::new("重置示例"))
                            .item(PopupMenuItem::new("查看状态"))
                    }),
            ),
        )
    }

    fn settings_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme();
        ContentPane::new(
            theme,
            div()
                .w_full()
                .max_w(px(760.))
                .p(px(32.))
                .child(page_title(self.tr("偏好设置", "Preferences"), "", theme))
                .child(desktop_settings_section_title(
                    self.tr("语言", "Language"),
                    true,
                ))
                .child(
                    desktop_settings_group(theme).child(desktop_settings_row(
                        self.tr("语言", "Language"),
                        self.tr(
                            "选择 Desktop UI 的显示语言。",
                            "Choose the language used by Desktop UI.",
                        ),
                        div()
                            .w(px(180.))
                            .flex_none()
                            .child(DesktopSelect::new(&self.language_select).w_full()),
                        theme,
                    )),
                )
                .child(desktop_settings_section_title(
                    self.tr("外观", "Appearance"),
                    false,
                ))
                .child(
                    desktop_settings_group(theme).child(desktop_settings_row(
                        self.tr("主题", "Theme"),
                        self.tr(
                            "跟随系统，或固定使用浅色与深色外观。",
                            "Follow the system or use a fixed light or dark appearance.",
                        ),
                        div()
                            .flex()
                            .gap(px(4.))
                            .child(
                                desktop_choice(
                                    "theme-system",
                                    self.tr("跟随系统", "System"),
                                    self.theme_preference == ThemePreference::System,
                                    theme,
                                )
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, window, cx| {
                                        this.set_theme(ThemePreference::System, window, cx)
                                    }),
                                ),
                            )
                            .child(
                                desktop_choice(
                                    "theme-light",
                                    self.tr("浅色", "Light"),
                                    self.theme_preference == ThemePreference::Light,
                                    theme,
                                )
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, window, cx| {
                                        this.set_theme(ThemePreference::Light, window, cx)
                                    }),
                                ),
                            )
                            .child(
                                desktop_choice(
                                    "theme-dark",
                                    self.tr("深色", "Dark"),
                                    self.theme_preference == ThemePreference::Dark,
                                    theme,
                                )
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, window, cx| {
                                        this.set_theme(ThemePreference::Dark, window, cx)
                                    }),
                                ),
                            ),
                        theme,
                    )),
                ),
        )
        .id("settings-content-scroll")
        .header(PaneHeader::new(self.tr("设置", "Settings"), theme))
    }

    fn set_theme(
        &mut self,
        preference: ThemePreference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.theme_preference = preference;
        let dark = match preference {
            ThemePreference::System => matches!(
                window.appearance(),
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
            ThemePreference::Light => false,
            ThemePreference::Dark => true,
        };
        self.dark = dark;
        ComponentTheme::change(
            if dark {
                ComponentThemeMode::Dark
            } else {
                ComponentThemeMode::Light
            },
            Some(window),
            cx,
        );
        apply_component_theme(self.theme(), cx);
        configure_window_material(window, dark);
        cx.notify();
    }
}

impl Render for PreviewApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme();
        let sidebar = self.sidebar(cx);
        let content = match self.page {
            PreviewPage::Layout if self.layout == PreviewLayout::ThreePane => {
                DesktopShell::three_pane(theme, sidebar, self.list_pane(), self.layout_content())
                    .into_any_element()
            }
            PreviewPage::Layout => {
                DesktopShell::two_pane(theme, sidebar, self.layout_content()).into_any_element()
            }
            PreviewPage::Controls => {
                DesktopShell::two_pane(theme, sidebar, self.controls_content(cx)).into_any_element()
            }
            PreviewPage::Settings => {
                DesktopShell::two_pane(theme, sidebar, self.settings_content(cx)).into_any_element()
            }
        };
        div()
            .size_full()
            .track_focus(&self.focus_handle)
            .child(content)
    }
}

fn page_title(
    title: &'static str,
    subtitle: &'static str,
    theme: DesktopTheme,
) -> impl IntoElement {
    div()
        .child(
            div()
                .text_size(px(24.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .when(!subtitle.is_empty(), |title| {
            title.child(
                div()
                    .mt(px(6.))
                    .text_color(theme.text_secondary)
                    .child(subtitle),
            )
        })
}

fn section_title(title: &'static str) -> impl IntoElement {
    div()
        .mt(px(32.))
        .text_size(px(14.))
        .font_weight(FontWeight::MEDIUM)
        .child(title)
}

fn component_section(title: &'static str) -> Div {
    div().flex().flex_col().gap(px(12.)).child(
        div()
            .text_size(px(14.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(title),
    )
}

fn field_label(label: &'static str, theme: DesktopTheme) -> impl IntoElement {
    div()
        .text_size(px(12.))
        .text_color(theme.text_secondary)
        .child(label)
}

fn swatch(label: &'static str, color: Hsla, theme: DesktopTheme) -> AnyElement {
    div()
        .w(px(128.))
        .h(px(88.))
        .rounded(px(RADIUS_CARD))
        .border_1()
        .border_color(theme.border)
        .bg(color)
        .p(px(12.))
        .flex()
        .items_end()
        .text_size(px(12.))
        .text_color(theme.text_secondary)
        .child(label)
        .into_any_element()
}

fn main() {
    gpui_platform::application()
        .with_assets(gpui_component_assets::Assets)
        .run(|cx: &mut App| {
            cx.set_app_identity("com.lankide.desktop-ui-preview", "Desktop UI Preview");
            gpui_component::init(cx);
            apply_component_theme(DesktopTheme::light(), cx);
            cx.open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some("Desktop UI Preview".into()),
                        appears_transparent: cfg!(target_os = "macos"),
                        traffic_light_position: cfg!(target_os = "macos")
                            .then(|| point(px(16.), px(17.))),
                        ..Default::default()
                    }),
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1180.), px(760.)),
                        cx,
                    ))),
                    window_min_size: Some(size(px(900.), px(620.))),
                    app_owns_titlebar_drag: cfg!(target_os = "macos"),
                    ..Default::default()
                },
                |window, cx| {
                    let view = cx.new(|cx| {
                        observe_window_frame(window, cx);
                        PreviewApp::new(window, cx)
                    });
                    configure_window_material(window, false);
                    cx.new(|cx| Root::new(view, window, cx).bordered(false))
                },
            )
            .expect("failed to open Desktop UI Preview");
        });
}
