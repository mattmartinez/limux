//! "Edit description…" on a workspace card: the same change as
//! `limux set-description`, made from the sidebar's context menu.

use super::*;

pub(super) fn show_workspace_description_dialog(state: &State, workspace_id: &str) {
    let (workspace_name, current) = {
        let app_state = state.borrow();
        let Some(workspace) = app_state
            .workspaces
            .iter()
            .find(|workspace| workspace.id == workspace_id)
        else {
            return;
        };
        (workspace.name.clone(), workspace.description.clone())
    };

    let dialog = gtk::Window::builder()
        .title("Workspace Description")
        .modal(true)
        .default_width(520)
        .resizable(false)
        .build();
    if let Some(window) = active_window(state) {
        dialog.set_transient_for(Some(&window));
    }

    let intro = gtk::Label::builder()
        .label(format!(
            "A line shown under {workspace_name} in the sidebar, for example what is still pending."
        ))
        .halign(gtk::Align::Start)
        .wrap(true)
        .build();
    let entry = gtk::Entry::builder()
        .text(current.as_deref().unwrap_or_default())
        .placeholder_text("For example: PROD DEPLOYS STILL PENDING")
        .max_length(workspace_card::MAX_DESCRIPTION_LEN as i32)
        .hexpand(true)
        .activates_default(true)
        .build();
    let hint = gtk::Label::builder()
        .label("Leave it empty to remove the description. Scripts can set it with `limux set-description`.")
        .halign(gtk::Align::Start)
        .wrap(true)
        .build();
    hint.add_css_class("dim-label");

    let buttons = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .halign(gtk::Align::End)
        .spacing(8)
        .build();
    let cancel_button = gtk::Button::with_label("Cancel");
    let save_button = gtk::Button::with_label("Save");
    save_button.add_css_class("suggested-action");
    buttons.append(&cancel_button);
    buttons.append(&save_button);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();
    content.append(&intro);
    content.append(&entry);
    content.append(&hint);
    content.append(&buttons);
    dialog.set_child(Some(&content));
    dialog.set_default_widget(Some(&save_button));

    let dialog_for_cancel = dialog.clone();
    cancel_button.connect_clicked(move |_| dialog_for_cancel.close());

    let dialog_for_key = dialog.clone();
    let key_controller = gtk::EventControllerKey::new();
    key_controller.connect_key_pressed(move |_controller, keyval, _keycode, _modifier| {
        if workspace_autostart_dialog_dismisses(keyval) {
            dialog_for_key.close();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    dialog.add_controller(key_controller);

    let state_for_save = state.clone();
    let workspace_id_for_save = workspace_id.to_string();
    let dialog_for_save = dialog.clone();
    let entry_for_save = entry.clone();
    save_button.connect_clicked(move |_| {
        let mut params = serde_json::Map::new();
        params.insert(
            "description".to_string(),
            serde_json::Value::String(entry_for_save.text().to_string()),
        );
        let request = match workspace_card::parse_card_request("workspace.set_description", &params)
        {
            Ok(request) => request,
            Err(message) => {
                eprintln!("limux: description not saved: {message}");
                return;
            }
        };
        let applied = {
            let mut app_state = state_for_save.borrow_mut();
            app_state
                .workspaces
                .iter_mut()
                .find(|workspace| workspace.id == workspace_id_for_save)
                .map(|workspace| apply_card_request(workspace, request).is_ok())
                .unwrap_or(false)
        };
        if applied {
            request_session_save(&state_for_save);
        }
        dialog_for_save.close();
    });

    dialog.present();
    entry.grab_focus();
}
