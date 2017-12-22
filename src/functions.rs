use ::*;

// BTW, only reason for pub(crate) is because it
// otherwise complains about publishing a private type.
// It's not like you can `extern crate` a program, can you?

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
    app.message_input.set_reveal_child(false);
    render_channels(None, app);
}
pub(crate) fn render_mode(container: &GtkBox, bitmask: u8) {
    for child in container.get_children() {
        container.remove(&child);
    }

    let mut check = CheckButton::new_with_label("Read messages");
    check.set_active(bitmask & common::PERM_READ == common::PERM_READ);
    container.add(&check);

    check = CheckButton::new_with_label("Write messages");
    check.set_active(bitmask & common::PERM_WRITE == common::PERM_WRITE);
    container.add(&check);

    check = CheckButton::new_with_label("Manage channel");
    check.set_active(bitmask & common::PERM_MANAGE_CHANNELS == common::PERM_MANAGE_CHANNELS);
    container.add(&check);

    check = CheckButton::new_with_label("Manage messages");
    check.set_active(bitmask & common::PERM_MANAGE_MESSAGES == common::PERM_MANAGE_MESSAGES);
    container.add(&check);

    check = CheckButton::new_with_label("Manage user modes");
    check.set_active(bitmask & common::PERM_MANAGE_MODES == common::PERM_MANAGE_MODES);
    container.add(&check);

    container.show_all();
    container.queue_draw();
}
pub(crate) fn get_mode(container: &GtkBox) -> Option<u8> {
    let mut children = container.get_children().into_iter();
    let mut bitmask = 0;

    if children.next()?.downcast::<CheckButton>().ok()?.get_active() {
        bitmask |= common::PERM_READ;
    }
    if children.next()?.downcast::<CheckButton>().ok()?.get_active() {
        bitmask |= common::PERM_WRITE;
    }
    if children.next()?.downcast::<CheckButton>().ok()?.get_active() {
        bitmask |= common::PERM_MANAGE_CHANNELS;
    }
    if children.next()?.downcast::<CheckButton>().ok()?.get_active() {
        bitmask |= common::PERM_MANAGE_MESSAGES;
    }
    if children.next()?.downcast::<CheckButton>().ok()?.get_active() {
        bitmask |= common::PERM_MANAGE_MODES;
    }

    Some(bitmask)
}
pub(crate) fn render_servers(app: &Rc<App>) {
    for child in app.servers.get_children() {
        app.servers.remove(&child);
    }
    let mut stmt = app.db.prepare("SELECT ip, name, hash, token FROM servers ORDER BY name").unwrap();
    let mut rows = stmt.query(&[]).unwrap();

    while let Some(row) = rows.next() {
        let row = row.unwrap();
        let addr: Rc<String> = Rc::new(row.get(0));
        let name: Rc<String> = Rc::new(row.get(1));
        let hash: Rc<String> = Rc::new(row.get(2));
        let token: Rc<Option<String>> = Rc::new(row.get(3));

        let ip_parsed = connections::parse_addr(&addr);

        let name_clone: Rc<String> = Rc::clone(&name);
        let hash_clone: Rc<String> = Rc::clone(&hash);
        let token_clone: Rc<Option<String>> = Rc::clone(&token);

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
            println!("server with ip {} was clicked", addr);
            app_clone.server_name.set_text(&name_clone);
            let mut ok = false;
            app_clone.connections.execute(addr, |result| {
                ok = result.is_ok();
            });
            if ok {
                app_clone.connections.set_current(Some(addr));
                app_clone.message_edit.set_reveal_child(false);
                render_channels(Some(addr), &app_clone);
            } else {
                connect(&app_clone, addr, (*hash_clone).clone(), (*token_clone).clone());
            }
        });

        let app_clone = Rc::clone(&app);
        button.connect_button_press_event(move |_, event| {
            if event.get_button() == 3 {
                let menu = Menu::new();

                let addr_clone: Rc<String> = Rc::clone(&addr);
                let name: Rc<String> = Rc::clone(&name);
                let hash: Rc<String> = Rc::clone(&hash);

                let edit = MenuItem::new_with_label("Edit server");
                let app_clone2 = Rc::clone(&app_clone);
                edit.connect_activate(move |_| {
                    app_clone2.stack_edit_server.name.set_text(&name);
                    app_clone2.stack_edit_server.server.set_text(&addr_clone);
                    app_clone2.stack_edit_server.server.set_sensitive(false);
                    app_clone2.stack_edit_server.hash.set_text(&hash);

                    app_clone2.stack.set_visible_child(&app_clone2.stack_edit_server.container);
                });
                menu.add(&edit);

                let disconnect = MenuItem::new_with_label("Disconnect server");

                let app_clone2 = Rc::clone(&app_clone);
                disconnect.connect_activate(move |_| {
                    if let Some(parsed) = ip_parsed {
                        app_clone2.connections.remove(parsed);
                        if *app_clone2.connections.current_server.lock().unwrap() == Some(parsed) {
                            deselect_server(&app_clone2);
                        }
                        render_servers(&app_clone2);
                    }
                });
                menu.add(&disconnect);

                let forget = MenuItem::new_with_label("Forget server");

                let app_clone2 = Rc::clone(&app_clone);
                let addr = Rc::clone(&addr);
                forget.connect_activate(move |_| {
                    app_clone2.db.execute("DELETE FROM servers WHERE ip = ?", &[&*addr]).unwrap();
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
            if result.is_err() { return; }
            let synac = result.unwrap();

            let mut channel_list: Vec<_> = synac.state.channels.values().collect();
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
                        if result.is_err() { return; }
                        let synac = result.unwrap();

                        let mut can_read = true; // Assume yes because usually that's the case
                        let mut can_write = true; // Assume yes because usually that's the case

                        if let Some(channel) = synac.state.channels.get(&channel_id) {
                            if let Some(user) = synac.state.users.get(&synac.user) {
                                let mode = synac::get_mode(&channel, &user);
                                can_read  = mode & common::PERM_READ == common::PERM_READ;
                                can_write = mode & common::PERM_WRITE == common::PERM_WRITE;
                            }
                        }

                        if !can_read {
                            alert(&app_clone.window, MessageType::Info, "You don't have permission to read this channel");
                            return;
                        }

                        app_clone.message_input.set_reveal_child(can_write);

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
                    });
                    render_messages(Some(addr), &app_clone);
                    render_users(Some(addr), &app_clone);
                });

                let app_clone = Rc::clone(&app);
                button.connect_button_press_event(move |_, event| {
                    if event.get_button() == 3 {
                        let menu = Menu::new();

                        let mut manage_channels = false;
                        let mut can_read = false;

                        app_clone.connections.execute(addr, |result| {
                            if result.is_err() { return; }
                            let synac = result.unwrap();

                            if let Some(channel) = synac.state.channels.get(&channel_id) {
                                if let Some(user) = synac.state.users.get(&synac.user) {
                                    let mode = synac::get_mode(&channel, &user);
                                    manage_channels = mode & common::PERM_MANAGE_CHANNELS == common::PERM_MANAGE_CHANNELS;
                                    can_read        = mode & common::PERM_READ == common::PERM_READ;
                                }
                            }
                        });

                        if !can_read {
                            return Inhibit(false);
                        }

                        if manage_channels {
                            let edit = MenuItem::new_with_label("Edit channel");

                            let app_clone1 = Rc::clone(&app_clone);
                            edit.connect_activate(move |_| {
                                app_clone1.connections.execute(addr, |result| {
                                    if result.is_err() { return; }
                                    let synac = result.unwrap();

                                    if let Some(channel) = synac.state.channels.get(&channel_id) {
                                        *app_clone1.stack_edit_channel.edit.borrow_mut() = Some(channel.id);
                                        app_clone1.stack_edit_channel.name.set_text(&channel.name);
                                        render_mode(&app_clone1.stack_edit_channel.mode_bots, channel.default_mode_bot);
                                        render_mode(&app_clone1.stack_edit_channel.mode_users, channel.default_mode_user);
                                    }
                                });

                                app_clone1.stack.set_visible_child(&app_clone1.stack_edit_channel.container);
                            });

                            menu.add(&edit);

                            let delete = MenuItem::new_with_label("Delete channel");

                            let app_clone2 = Rc::clone(&app_clone);
                            delete.connect_activate(move |_| {
                                app_clone2.connections.execute(addr, |result| {
                                    if result.is_err() { return; }
                                    let synac = result.unwrap();

                                    let result = synac.session.send(&Packet::ChannelDelete(common::ChannelDelete {
                                        id: channel_id
                                    }));
                                    if let Err(err) = result {
                                        eprintln!("failed to send packet: {}", err);
                                    }
                                });
                            });

                            menu.add(&delete);
                        }

                        menu.show_all();
                        menu.popup_at_pointer(&**event);
                    }
                    Inhibit(false)
                });
                app.channels.add(&button);
            }
        });
    } else {
        app.channel_name.set_text("");
        render_messages(None, &app);
        render_users(None, &app);
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

            let mut last: Option<&common::Message> = None;

            for msg in synac.messages.get(channel) {
                let msgbox = GtkBox::new(Orientation::Vertical, 2);
                let authorbox = GtkBox::new(Orientation::Horizontal, 4);

                if last.map(|msg| msg.author) != Some(msg.author)
                    || last.map(|msg| msg.timestamp + 60*5) < Some(msg.timestamp) {
                    if last.is_some() {
                        app.messages.add(&Separator::new(Orientation::Vertical));
                    }

                    let author = Label::new(&*synac.state.users[&msg.author].name);
                    author.set_xalign(0.0);
                    add_class(&author, "author");
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
                    time.set_margin_right(10);
                    time.set_hexpand(true);
                    time.set_xalign(1.0);
                    add_class(&time, "time");
                    authorbox.add(&time);

                    msgbox.add(&authorbox);
                }

                let string = String::from_utf8_lossy(&msg.text).into_owned();
                let text = Label::new(&*string);
                text.set_line_wrap(true);
                text.set_selectable(true);
                text.set_xalign(0.0);

                let app_clone = Rc::clone(&app);
                let msg_id = msg.id;
                let msg_mine = msg.author == synac.user;

                text.connect_populate_popup(move |_, menu| {
                    menu.add(&SeparatorMenuItem::new());

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
                                    has_perms = synac::get_mode(&channel, &user) & common::PERM_MANAGE_MESSAGES
                                                    == common::PERM_MANAGE_MESSAGES;
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
                });

                msgbox.add(&text);
                app.messages.add(&msgbox);

                last = Some(msg);
            }
        });
    }
    app.messages.show_all();
    app.messages.queue_draw();
}
pub(crate) fn render_users(addr: Option<SocketAddr>, app: &Rc<App>) {
    for child in app.users.get_children() {
        app.users.remove(&child);
    }
    if let Some(addr) = addr {
        app.connections.execute(addr, |result| {
            if result.is_err() { return; }
            let synac = result.unwrap();

            let channel = synac.current_channel.and_then(|id| synac.state.channels.get(&id));
            if channel.is_none() { return; }
            let channel = channel.unwrap();

            let draw = |user: &common::User| {
                let label = Label::new(&*user.name);
                let event = EventBox::new();
                event.add(&label);

                let user_id = user.id;
                let app_clone = Rc::clone(&app);
                event.connect_button_press_event(move |_, event| {
                    if event.get_button() != 3 {
                        return Inhibit(false);
                    }
                    let menu = Menu::new();

                    let edit_mode = MenuItem::new_with_label("Edit mode");

                    let app_clone = Rc::clone(&app_clone);
                    edit_mode.connect_activate(move |_| {
                        app_clone.connections.execute(addr, |result| {
                            if result.is_err() { return; }
                            let synac = result.unwrap();

                            let channel = synac.current_channel.and_then(|id| synac.state.channels.get(&id));
                            let user = synac.state.users.get(&user_id);

                            if let Some(channel) = channel {
                                if let Some(user) = user {
                                    *app_clone.stack_edit_user.user.borrow_mut() = Some(user_id);
                                    if user.modes.contains_key(&channel.id) {
                                        app_clone.stack_edit_user.radio_some.set_active(true);
                                        app_clone.stack_edit_user.mode.set_sensitive(true);
                                    } else {
                                        app_clone.stack_edit_user.radio_none.set_active(true);
                                        app_clone.stack_edit_user.mode.set_sensitive(false);
                                    }
                                    let mode = synac::get_mode(&channel, &user);
                                    render_mode(&app_clone.stack_edit_user.mode, mode);

                                    app_clone.stack.set_visible_child(&app_clone.stack_edit_user.container);
                                }
                            }
                        });
                    });

                    menu.add(&edit_mode);

                    menu.show_all();
                    menu.popup_at_pointer(&**event);
                    Inhibit(false)
                });

                app.users.add(&event);
                app.users.add(&Separator::new(Orientation::Vertical));
            };

            let mut users: Vec<_> = synac.state.users.values().collect();
            users.sort_by_key(|user| &user.name);

            let label = Label::new("This channel:");
            add_class(&label, "bold");
            label.set_xalign(0.0);
            app.users.add(&label);

            // Don't worry, the following .cloned()s just copies the reference

            users.iter().cloned().filter(|user| {
                !user.ban && synac::get_mode(&channel, &user) & common::PERM_READ == common::PERM_READ
            }).for_each(&draw);

            let label = Label::new("Other:");
            add_class(&label, "bold");
            label.set_xalign(0.0);
            app.users.add(&label);

            users.iter().cloned().filter(|user| {
                !user.ban && synac::get_mode(&channel, &user) & common::PERM_READ != common::PERM_READ
            }).for_each(&draw);

            let label = Label::new("Banned:");
            add_class(&label, "bold");
            label.set_xalign(0.0);
            app.users.add(&label);

            users.iter().cloned().filter(|user| user.ban).for_each(&draw);
        });
    }
    app.users.show_all();
    app.users.queue_draw();
}
