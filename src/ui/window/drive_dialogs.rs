// SPDX-License-Identifier: MIT

//! Removable-drive dialogs: Format, Rename (volume label), Properties.

use std::{cell::Cell, rc::Rc};

use gtk::{gio, glib, prelude::*};

use crate::{
    assets,
    ui::{
        controls::{ModalLayout, form_check_button, form_entry, form_label, modal_layout},
        modal::{ModalHost, dismiss_modal_layer, modal_layer},
    },
};

use super::drive_ops::{self, FilesystemType};

struct ModalShell {
    layout: ModalLayout,
    layer: gtk::Box,
    overlay: gtk::Overlay,
    blurred_root: Option<crate::ui::blur::BlurBin>,
}

fn modal_shell(
    parent: &gtk::Widget,
    icon: &str,
    title: &str,
    subtitle: &str,
    confirm_label: &str,
    danger: bool,
) -> Option<ModalShell> {
    let host = ModalHost::blurred_for(parent)?;
    let layout = modal_layout(icon, title, subtitle, "Cancel");
    if danger {
        layout.content.add_css_class("destructive");
    }
    layout.confirm.set_label(confirm_label);
    layout.confirm.add_css_class("suggested-action");
    let layer = modal_layer(
        &layout.content,
        &host.overlay,
        host.blurred_root.clone(),
        Some(Rc::new(|| true)),
    );
    host.overlay.add_overlay(&layer);
    Some(ModalShell {
        layout,
        layer,
        overlay: host.overlay,
        blurred_root: host.blurred_root,
    })
}

fn field_block(label_text: &str, field: &impl IsA<gtk::Widget>) -> gtk::Box {
    let block = gtk::Box::new(gtk::Orientation::Vertical, 4);
    block.set_margin_top(8);
    block.set_margin_start(16);
    block.set_margin_end(16);
    let label = form_label(label_text);
    label.set_xalign(0.0);
    block.append(&label);
    field.set_hexpand(true);
    block.append(field);
    block
}

fn inline_error() -> gtk::Label {
    let label = gtk::Label::new(None);
    label.add_css_class("form-error");
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_margin_start(16);
    label.set_margin_end(16);
    label.set_visible(false);
    label
}

fn show_inline_error(label: &gtk::Label, message: &str) {
    label.set_text(message);
    label.set_visible(true);
}

fn human_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn refresh_format_validity(
    available: &[FilesystemType],
    fs_combo: &gtk::DropDown,
    label_entry: &gtk::Entry,
    limit_hint: &gtk::Label,
    confirm_btn: &gtk::Button,
) {
    let Some(fs_type) = available.get(fs_combo.selected() as usize).copied() else {
        confirm_btn.set_sensitive(false);
        return;
    };
    limit_hint.set_text(&format!(
        "{} labels hold at most {} characters.",
        fs_type.label(),
        fs_type.max_label_len()
    ));
    confirm_btn.set_sensitive(label_entry.text().chars().count() <= fs_type.max_label_len());
}

/// Wire Cancel and Escape. Each dialog connects its own confirm button.
fn wire_modal_close(shell: &ModalShell) {
    let layer = shell.layer.clone();
    let overlay = shell.overlay.clone();
    let root = shell.blurred_root.clone();
    let close: Rc<dyn Fn()> = Rc::new(move || {
        dismiss_modal_layer(&layer, &overlay, root.as_ref());
    });
    let cancelled = close.clone();
    shell.layout.cancel.connect_clicked(move |_| cancelled());
    let closed = close.clone();
    shell.layout.close.connect_clicked(move |_| closed());
    let escaped = close.clone();
    let escape = gtk::EventControllerKey::new();
    escape.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            escaped();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    shell.layer.add_controller(escape);
    shell.layout.confirm.grab_focus();
}

/// Properties: name, device, filesystem, mount status, capacity bar.
/// Everything is read synchronously; unavailable data shows as em dash.
pub(super) fn show_drive_properties(parent: &gtk::Widget, volume: &gio::Volume) {
    let name = volume.name().to_string();
    let volume = volume.clone();
    let Some(shell) = modal_shell(
        parent,
        assets::icons::INFO,
        "Properties",
        &name,
        "Close",
        false,
    ) else {
        return;
    };

    let grid = gtk::Grid::new();
    grid.set_column_spacing(12);
    grid.set_row_spacing(8);
    grid.set_margin_top(8);
    grid.set_margin_start(16);
    grid.set_margin_end(16);
    grid.set_margin_bottom(8);
    grid.set_hexpand(true);
    grid.set_column_homogeneous(false);

    let mut row = 0;
    let mut add_row = |label_text: &str, value_text: &str| {
        let label = form_label(label_text);
        label.set_xalign(0.0);
        let value = gtk::Label::new(Some(value_text));
        value.set_xalign(0.0);
        value.set_hexpand(true);
        value.set_selectable(true);
        value.set_ellipsize(gtk::pango::EllipsizeMode::End);
        grid.attach(&label, 0, row, 1, 1);
        grid.attach(&value, 1, row, 1, 1);
        row += 1;
    };

    add_row("Name", &name);
    let block_device = drive_ops::block_device_for_volume(&volume);
    add_row(
        "Device",
        &block_device
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "—".to_owned()),
    );

    let filesystem = block_device
        .as_ref()
        .and_then(|device| drive_ops::filesystem_label_for_device(device))
        .unwrap_or_else(|| "Unknown".to_owned());
    add_row("Filesystem", &filesystem);

    let total_bytes = block_device
        .as_ref()
        .and_then(|device| drive_ops::device_size_bytes(device));
    match volume.get_mount() {
        Some(mount) => {
            let location = mount
                .root()
                .path()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "—".to_owned());
            add_row("Mount point", &location);
            add_row("Status", "Mounted");
            let usage = mount
                .root()
                .path()
                .and_then(|root| drive_ops::usage_for_path(&root));
            let total = usage
                .map(|(total, _)| total)
                .filter(|total| *total > 0)
                .or(total_bytes);
            match (total, usage) {
                (Some(total), Some((_, available))) => {
                    let used = total.saturating_sub(available);
                    add_row("Capacity", &human_size(total));
                    add_row("Used", &human_size(used));
                    add_row("Free", &human_size(available));
                    let bar = gtk::ProgressBar::new();
                    bar.set_fraction(used as f64 / total as f64);
                    bar.set_show_text(true);
                    bar.set_text(Some(&format!(
                        "{} of {} used",
                        human_size(used),
                        human_size(total)
                    )));
                    bar.set_margin_top(8);
                    grid.attach(&bar, 0, row, 2, 1);
                }
                (Some(total), None) => {
                    add_row("Capacity", &human_size(total));
                    add_row("Used", "Unavailable");
                }
                (None, _) => add_row("Capacity", "Unavailable"),
            }
        }
        None => {
            add_row("Status", "Not mounted");
            if let Some(total) = total_bytes {
                add_row("Capacity", &human_size(total));
            }
        }
    }

    shell.layout.body.append(&grid);

    shell.layout.confirm.set_visible(false);
    shell.layout.cancel.set_visible(false);
    wire_modal_close(&shell);
}

pub(super) fn show_rename_dialog(parent: &gtk::Widget, volume: &gio::Volume) {
    let name = volume.name().to_string();
    let volume = volume.clone();
    let Some(shell) = modal_shell(
        parent,
        assets::icons::PENCIL,
        "Rename Volume",
        &name,
        "Rename",
        false,
    ) else {
        return;
    };

    let current = gtk::Label::new(Some(&name));
    current.set_xalign(0.0);
    current.set_ellipsize(gtk::pango::EllipsizeMode::End);
    shell
        .layout
        .body
        .append(&field_block("Current label", &current));

    let entry = form_entry();
    entry.set_placeholder_text(Some("New volume label"));
    entry.set_text(&name);
    entry.select_region(0, -1);
    shell.layout.body.append(&field_block("New label", &entry));

    // Label limits differ per filesystem (FAT32: 11 chars). Detect upfront
    // so overlong labels are rejected here
    let detected_fs = drive_ops::block_device_for_volume(&volume)
        .map(|device| drive_ops::filesystem_of_device(&device));
    let max_len = detected_fs.map(|fs| fs.max_label_len()).unwrap_or(11);
    entry.set_max_length(max_len as i32);
    let fs_info = gtk::Label::new(Some(&match detected_fs {
        Some(fs) => format!(
            "Filesystem: {} — labels hold at most {} characters",
            fs.label(),
            fs.max_label_len()
        ),
        None => "Filesystem: unknown — keeping the label short is safest".to_owned(),
    }));
    fs_info.add_css_class("dim-label");
    fs_info.set_xalign(0.0);
    fs_info.set_wrap(true);
    fs_info.set_margin_start(16);
    fs_info.set_margin_end(16);
    shell.layout.body.append(&fs_info);

    // The label tools need exclusive access, so a mounted volume is
    // unmounted first. Only say so when that actually applies.
    if volume.get_mount().is_some() {
        let hint = gtk::Label::new(Some(
            "This volume is currently mounted. It will be unmounted to apply the new label; click it in the sidebar afterwards to mount it again.",
        ));
        hint.add_css_class("dim-label");
        hint.set_xalign(0.0);
        hint.set_wrap(true);
        hint.set_margin_start(16);
        hint.set_margin_end(16);
        shell.layout.body.append(&hint);
    }

    let error = inline_error();
    shell.layout.body.append(&error);

    let confirm = shell.layout.confirm.clone();
    confirm.set_sensitive(false);
    {
        let confirm = confirm.clone();
        let current_name = name.clone();
        entry.connect_changed(move |entry| {
            let text = entry.text();
            confirm.set_sensitive(
                !text.trim().is_empty() && text != current_name && text.chars().count() <= max_len,
            );
        });
    }

    let shell_layer = shell.layer.clone();
    let shell_overlay = shell.overlay.clone();
    let shell_root = shell.blurred_root.clone();
    let parent = parent.clone();
    let fired = Rc::new(Cell::new(false));
    shell.layout.confirm.connect_clicked(move |_| {
        if fired.get() {
            return;
        }
        let new_label = entry.text().to_string();
        if new_label.trim().is_empty() {
            show_inline_error(&error, "The label cannot be empty.");
            return;
        }
        if new_label == name {
            show_inline_error(&error, "Enter a label different from the current one.");
            return;
        }
        if new_label.chars().count() > max_len {
            show_inline_error(
                &error,
                &format!("Labels on this volume hold at most {max_len} characters."),
            );
            return;
        }
        fired.set(true);
        dismiss_modal_layer(&shell_layer, &shell_overlay, shell_root.as_ref());
        let display = name.clone();
        let task_parent = parent.clone();
        let task_volume = volume.clone();
        drive_ops::spawn_drive_task(task_parent.clone(), display, move || {
            let volume = task_volume.clone();
            let new_label = new_label.clone();
            async move { drive_ops::rename_volume(task_parent.clone(), volume, new_label).await }
        });
    });
    wire_modal_close_except_confirm(&shell);
}

/// Format in two steps: configure on step one, then review a summary and
/// press the confirm button a second time.
pub(super) fn show_format_dialog(parent: &gtk::Widget, volume: &gio::Volume) {
    let name = volume.name().to_string();
    let device = drive_ops::block_device_for_volume(volume)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "unknown device".to_owned());
    let subtitle = format!("{name} ({device})");
    let volume = volume.clone();
    let Some(shell) = modal_shell(
        parent,
        assets::icons::TRIANGLE_ALERT,
        "Format Drive",
        &subtitle,
        "Continue",
        true,
    ) else {
        return;
    };

    let available: Vec<FilesystemType> = [
        FilesystemType::Fat32,
        FilesystemType::Ntfs,
        FilesystemType::Exfat,
    ]
    .into_iter()
    .filter(|fs| fs.available())
    .collect();
    if available.is_empty() {
        let missing = gtk::Label::new(Some(
            "No formatting tools found. Install dosfstools, ntfs-3g, or exfatprogs to format drives.",
        ));
        missing.set_wrap(true);
        missing.set_margin_start(16);
        missing.set_margin_end(16);
        shell.layout.body.append(&missing);
        shell.layout.confirm.set_sensitive(false);
        wire_modal_close(&shell);
        return;
    }

    // Configuration
    let step1 = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let options: Vec<&str> = available
        .iter()
        .map(|fs| {
            let label = format!("{} \u{2014} {}", fs.label(), fs.description());
            Box::leak(label.into_boxed_str()) as &str
        })
        .collect();
    let fs_combo = gtk::DropDown::from_strings(&options);
    fs_combo.set_selected(0);
    step1.append(&field_block("Filesystem", &fs_combo));

    let label_entry = form_entry();
    label_entry.set_placeholder_text(Some("Volume label (optional)"));
    label_entry.set_max_length(32);
    step1.append(&field_block("Label", &label_entry));

    let limit_hint = gtk::Label::new(None);
    limit_hint.add_css_class("dim-label");
    limit_hint.set_xalign(0.0);
    limit_hint.set_wrap(true);
    limit_hint.set_margin_start(16);
    limit_hint.set_margin_end(16);
    step1.append(&limit_hint);

    let quick_check = form_check_button("Quick format");
    quick_check.set_active(true);
    let check_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    check_row.set_margin_top(8);
    check_row.set_margin_start(16);
    check_row.append(&quick_check);
    step1.append(&check_row);

    if volume.get_mount().is_some() {
        let mount_note = gtk::Label::new(Some(
            "This volume is currently mounted. It will be unmounted to format it; click it in the sidebar afterwards to mount it again.",
        ));
        mount_note.add_css_class("dim-label");
        mount_note.set_xalign(0.0);
        mount_note.set_wrap(true);
        mount_note.set_margin_top(8);
        mount_note.set_margin_start(16);
        mount_note.set_margin_end(16);
        step1.append(&mount_note);
    }
    shell.layout.body.append(&step1);

    // Explicit confirmation
    let step2 = gtk::Box::new(gtk::Orientation::Vertical, 0);
    step2.set_visible(false);
    let summary = gtk::Label::new(None);
    summary.set_xalign(0.0);
    summary.set_wrap(true);
    summary.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    summary.set_margin_top(8);
    summary.set_margin_start(16);
    summary.set_margin_end(16);
    summary.set_margin_bottom(8);
    step2.append(&summary);
    let warning = gtk::Label::new(Some(
        "This permanently erases ALL DATA on this volume. This cannot be undone.",
    ));
    warning.add_css_class("form-message");
    warning.add_css_class("error");
    warning.set_xalign(0.0);
    warning.set_wrap(true);
    warning.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    warning.set_margin_top(8);
    warning.set_margin_start(16);
    warning.set_margin_end(16);
    warning.set_margin_bottom(8);
    step2.append(&warning);
    shell.layout.body.append(&step2);

    let confirm_btn = shell.layout.confirm.clone();
    {
        let available = available.clone();
        let fs_combo = fs_combo.clone();
        let label_entry = label_entry.clone();
        let limit_hint = limit_hint.clone();
        let confirm_btn = confirm_btn.clone();
        label_entry.clone().connect_changed(move |_| {
            refresh_format_validity(
                &available,
                &fs_combo,
                &label_entry,
                &limit_hint,
                &confirm_btn,
            );
        });
    }
    {
        let available = available.clone();
        let fs_combo = fs_combo.clone();
        let label_entry = label_entry.clone();
        let limit_hint = limit_hint.clone();
        let confirm_btn = confirm_btn.clone();
        fs_combo
            .clone()
            .connect_notify_local(Some("selected"), move |_, _| {
                refresh_format_validity(
                    &available,
                    &fs_combo,
                    &label_entry,
                    &limit_hint,
                    &confirm_btn,
                );
            });
    }
    refresh_format_validity(
        &available,
        &fs_combo,
        &label_entry,
        &limit_hint,
        &confirm_btn,
    );

    let shell_layer = shell.layer.clone();
    let shell_overlay = shell.overlay.clone();
    let shell_root = shell.blurred_root.clone();
    let parent = parent.clone();
    let armed = Rc::new(Cell::new(false));
    let fired = Rc::new(Cell::new(false));
    shell.layout.confirm.connect_clicked(move |_| {
        if fired.get() {
            return;
        }
        if !armed.get() {
            let Some(fs_type) = available.get(fs_combo.selected() as usize).copied() else {
                return;
            };
            let label = label_entry.text().to_string();
            if label.chars().count() > fs_type.max_label_len() {
                return;
            }
            let size = drive_ops::block_device_for_volume(&volume)
                .and_then(|device| drive_ops::device_size_bytes(&device))
                .map(human_size)
                .unwrap_or_else(|| "unknown size".to_owned());
            summary.set_text(&format!(
                "Drive: {} ({}, {})\nFilesystem: {}\nLabel: {}\nMode: {}",
                name,
                device,
                size,
                fs_type.label(),
                if label.is_empty() { "(none)" } else { &label },
                if quick_check.is_active() {
                    "Quick format"
                } else {
                    "Full format"
                },
            ));
            step1.set_visible(false);
            step2.set_visible(true);
            confirm_btn.set_label("Format");
            armed.set(true);
            return;
        }
        fired.set(true);
        dismiss_modal_layer(&shell_layer, &shell_overlay, shell_root.as_ref());
        let Some(fs_type) = available.get(fs_combo.selected() as usize).copied() else {
            return;
        };
        let label = label_entry.text().to_string();
        let quick = quick_check.is_active();
        let task_parent = parent.clone();
        let task_volume = volume.clone();
        let display = name.clone();
        drive_ops::spawn_drive_task(task_parent.clone(), display, move || {
            let volume = task_volume.clone();
            let label = label.clone();
            async move {
                drive_ops::format_volume(task_parent.clone(), volume, fs_type, label, quick).await
            }
        });
    });
    wire_modal_close_except_confirm(&shell);
}

/// Wire Cancel and Escape without touching the confirm button, which each
/// dialog connects itself.
fn wire_modal_close_except_confirm(shell: &ModalShell) {
    let layer = shell.layer.clone();
    let overlay = shell.overlay.clone();
    let root = shell.blurred_root.clone();
    let close: Rc<dyn Fn()> = Rc::new(move || {
        dismiss_modal_layer(&layer, &overlay, root.as_ref());
    });
    let cancelled = close.clone();
    shell.layout.cancel.connect_clicked(move |_| cancelled());
    let closed = close.clone();
    shell.layout.close.connect_clicked(move |_| closed());
    let escaped = close.clone();
    let escape = gtk::EventControllerKey::new();
    escape.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            escaped();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    shell.layer.add_controller(escape);
    shell.layout.confirm.grab_focus();
}
