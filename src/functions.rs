use ::*;

pub(crate) fn add_class<T: WidgetExt>(widget: &T, class: &str) {
    widget.get_style_context().and_then(|context| {
        context.add_class(class);
        Some(())
    });
}
pub(crate) fn alert(window: &Window, kind: MessageType, message: &str) {
    let dialog = MessageDialog::new(
        Some(window),
        DialogFlags::MODAL,
        kind,
        ButtonsType::Ok,
        message
    );
    dialog.connect_response(|dialog, _| dialog.destroy());
    dialog.show_all();
}
pub(crate) fn connect(app: &Rc<App>, addr: SocketAddr, hash: String, token: Option<String>)
    -> Option<Error>
{
    let result = app.connections.connect(addr, hash, token, || {
        let dialog = Dialog::new_with_buttons(
            Some("Synac: Password dialog"),
            Some(&app.window),
            DialogFlags::MODAL,
            &[("Ok", ResponseType::Ok.into())]
        );

        let content = dialog.get_content_area();
        content.add(&Label::new("Password:"));
        let entry = Entry::new();
        entry.set_input_purpose(InputPurpose::Password);
        entry.set_visibility(false);
        content.add(&entry);

        dialog.show_all();
        dialog.run();
        let text = entry.get_text().unwrap_or_default();
        dialog.destroy();
        Some((text, Rc::clone(&app.db)))
    });
    match result {
        Ok(synac) => {
            app.connections.insert(addr, synac);
            app.connections.set_current(Some(addr));
            app.message_edit.set_reveal_child(false);
            render_channels(Some(addr), app);
            None
        },
        Err(err) => {
            deselect_server(app);
            alert(&app.window, MessageType::Error, &format!("connection error: {}", err));
            Some(err)
        }
    }
}
pub(crate) fn deselect_server(app: &Rc<App>) {
    app.connections.set_current(None);
    app.server_name.set_text("");
    app.message_edit.set_reveal_child(false);
    render_channels(None, app);
}
pub(crate) fn render_servers(app: &Rc<App>) {
    for child in app.servers.get_children() {
        app.servers.remove(&child);
    }
    let mut stmt = app.db.prepare("SELECT ip, name, hash, token FROM servers ORDER BY name").unwrap();
    let mut rows = stmt.query(&[]).unwrap();

    while let Some(row) = rows.next() {
        let row = row.unwrap();
        let addr: String = row.get(0);
        let name: String = row.get(1);
        let hash: String = row.get(2);
        let token: Option<String> = row.get(3);

        let ip_parsed = connections::parse_addr(&addr);

        let button = Button::new_with_label(&name);
        let app_clone = Rc::clone(&app);
        button.connect_clicked(move |_| {
            let addr = match ip_parsed {
                Some(addr) => addr,
                None => {
                    alert(&app_clone.window, MessageType::Error, "Failed to parse IP address. Format: <ip[:port]>");
                    return;
                }
            };
            println!("Server with IP {} was clicked", addr);
            app_clone.server_name.set_text(&name);
            let mut ok = false;
            app_clone.connections.execute(addr, |result| {
                ok = result.is_ok();
            });
            if ok {
                app_clone.connections.set_current(Some(addr));
                app_clone.message_edit.set_reveal_child(false);
                render_channels(Some(addr), &app_clone);
            } else {
                connect(&app_clone, addr, hash.clone(), token.clone());
            }
        });

        let app_clone = Rc::clone(&app);
        button.connect_button_press_event(move |_, event| {
            if event.get_button() == 3 {
                let menu = Menu::new();

                let disconnect = MenuItem::new_with_label("Disconnect server");

                let app_clone1 = Rc::clone(&app_clone);
                disconnect.connect_activate(move |_| {
                    if let Some(parsed) = ip_parsed {
                        app_clone1.connections.remove(parsed);
                        if *app_clone1.connections.current_server.lock().unwrap() == Some(parsed) {
                            deselect_server(&app_clone1);
                        }
                        render_servers(&app_clone1);
                    }
                });
                menu.add(&disconnect);

                let forget = MenuItem::new_with_label("Forget server");

                let app_clone2 = Rc::clone(&app_clone);
                let addr_clone = addr.clone();
                forget.connect_activate(move |_| {
                    app_clone2.db.execute("DELETE FROM servers WHERE ip = ?", &[&addr_clone]).unwrap();
                    if let Some(parsed) = ip_parsed {
                        app_clone2.connections.remove(parsed);
                        if *app_clone2.connections.current_server.lock().unwrap() == Some(parsed) {
                            deselect_server(&app_clone2);
                        }
                    }
                    render_servers(&app_clone2);
                });
                menu.add(&forget);

                menu.show_all();
                menu.popup_at_pointer(Some(&**event));
            }
            Inhibit(false)
        });
        app.servers.add(&button);
    }
    app.servers.show_all();
    app.servers.queue_draw();
}
pub(crate) fn render_channels(addr: Option<SocketAddr>, app: &Rc<App>) {
    for child in app.channels.get_children() {
        app.channels.remove(&child);
    }
    if let Some(addr) = addr {
        app.connections.execute(addr, |result| {
            if let Ok(server) = result {
                let mut channel_list: Vec<_> = server.state.channels.values().collect();
                channel_list.sort_by_key(|channel| &channel.name);
                for channel in channel_list {
                    let mut name = String::with_capacity(channel.name.len() + 1);
                    name.push('#');
                    name.push_str(&channel.name);

                    let button = Button::new_with_label(&name);

                    let channel_id = channel.id;

                    let app_clone = Rc::clone(&app);
                    button.connect_clicked(move |_| {
                        app_clone.connections.execute(addr, |result| {
                            if let Ok(synac) = result {
                                synac.current_channel = Some(channel_id);
                                app_clone.channel_name.set_text(&name);
                                if !synac.messages.has(channel_id) {
                                    if let Err(err) = synac.session.send(&Packet::MessageList(common::MessageList {
                                        after: None,
                                        before: None,
                                        channel: channel_id,
                                        limit: common::LIMIT_BULK
                                    })) {
                                        eprintln!("error sending packet: {}", err);
                                    }
                                }
                            }
                        });
                        render_messages(Some(addr), &app_clone);
                    });
                    app.channels.add(&button);
                }
            }
        });
    } else {
        app.channel_name.set_text("");
        render_messages(None, &app);
    }

    app.channels.show_all();
    app.channels.queue_draw();
}
pub(crate) fn render_messages(addr: Option<SocketAddr>, app: &Rc<App>) {
    for child in app.messages.get_children() {
        app.messages.remove(&child);
    }
    if let Some(addr) = addr {
        app.connections.execute(addr, |result| {
            if result.is_err() { return; }
            let synac = result.unwrap();

            if synac.current_channel.is_none() { return };
            let channel = synac.current_channel.unwrap();

            for msg in synac.messages.get(channel) {
                let msgbox = GtkBox::new(Orientation::Vertical, 2);
                let authorbox = GtkBox::new(Orientation::Horizontal, 4);

                let author = Label::new(&*synac.state.users[&msg.author].name);
                author.set_xalign(0.0);
                authorbox.add(&author);

                authorbox.add(&Separator::new(Orientation::Horizontal));

                let mut time = String::with_capacity(32); // just a guess
                messages::format(&mut time, msg.timestamp);
                if let Some(edit) = msg.timestamp_edit {
                    time.push_str(" (edited ");
                    messages::format(&mut time, edit);
                    time.push(')');
                }
                let time = Label::new(&*time);
                time.set_xalign(0.0);
                authorbox.add(&time);

                msgbox.add(&authorbox);

                let string = String::from_utf8_lossy(&msg.text).into_owned();
                let text = Label::new(&*string);
                text.set_selectable(true);
                text.set_xalign(0.0);

                let event = EventBox::new();
                event.add(&text);

                let app_clone = Rc::clone(&app);
                let msg_id = msg.id;
                let msg_mine = msg.author == synac.user;

                event.connect_button_press_event(move |_, event| {
                    if event.get_button() == 3 {
                        let menu = Menu::new();

                        let mut has_perms = false;

                        if msg_mine {
                            has_perms = true;

                            let edit = MenuItem::new_with_label("Edit message");

                            let app_clone = Rc::clone(&app_clone);
                            let string = string.clone();
                            edit.connect_activate(move |_| {
                                *app_clone.message_edit_id.borrow_mut() = Some(msg_id);
                                app_clone.message_edit_input.set_text(&string);
                                app_clone.message_edit.set_reveal_child(true);
                            });

                            menu.add(&edit);
                        } else {
                            app_clone.connections.execute(addr, |result| {
                                if result.is_err() { return; }
                                let synac = result.unwrap();

                                if synac.current_channel.is_none() { return };
                                let channel_id = synac.current_channel.unwrap();

                                if let Some(channel) = synac.state.channels.get(&channel_id) {
                                    if let Some(user) = synac.state.users.get(&synac.user) {
                                        has_perms = synac::get_perm(&channel, &user) & common::PERM_MANAGE_MODES
                                                        == common::PERM_MANAGE_MODES;
                                    }
                                }
                            });
                        }

                        if has_perms {
                            let delete = MenuItem::new_with_label("Delete message");

                            let app_clone = Rc::clone(&app_clone);
                            delete.connect_activate(move |_| {
                                app_clone.connections.execute(addr, |result| {
                                    if result.is_err() { return; }
                                    let synac = result.unwrap();

                                    let result = synac.session.send(&Packet::MessageDelete(common::MessageDelete {
                                        id: msg_id
                                    }));
                                    if let Err(err) = result {
                                        eprintln!("error sending packet: {}", err);
                                    }
                                });
                            });

                            menu.add(&delete);
                        }

                        menu.show_all();
                        menu.popup_at_pointer(Some(&**event));
                    }
                    Inhibit(false)
                });

                msgbox.add(&event);

                app.messages.add(&msgbox);

                app.messages.add(&Separator::new(Orientation::Vertical));
            }
        });
    }
    app.messages.show_all();
    app.messages.queue_draw();
}
