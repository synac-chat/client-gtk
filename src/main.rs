#[macro_use] extern crate failure;
extern crate chrono;
extern crate gdk;
extern crate gtk;
extern crate rusqlite;
extern crate synac;
extern crate xdg;

mod connections;
mod messages;

use failure::Error;
use gdk::Screen;
use gtk::prelude::*;
use gtk::{
    Align,
    Box as GtkBox,
    Button,
    ButtonsType,
    CssProvider,
    Dialog,
    DialogFlags,
    Entry,
    InputPurpose,
    Label,
    Menu,
    MenuItem,
    MessageDialog,
    MessageType,
    Orientation,
    PositionType,
    ResponseType,
    ScrolledWindow,
    Separator,
    Stack,
    StyleContext,
    STYLE_PROVIDER_PRIORITY_APPLICATION,
    Window,
    WindowType
};
use connections::{Connections, Synac};
use rusqlite::Connection as SqlConnection;
use std::env;
use std::io::{Error as IoError, ErrorKind as IoErrorKind};
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::Arc;
use synac::common::{self, Packet};
use xdg::BaseDirectories;

#[derive(Debug, Fail)]
#[fail(display = "sadly GTK+ doesn't support unicode paths")]
struct UnicodePathError;

fn main() {
    let basedirs = match BaseDirectories::with_prefix("synac") {
        Ok(basedirs) => basedirs,
        Err(err) => { eprintln!("error initializing xdg: {}", err); return; }
    };
    let path = match basedirs.find_data_file("data.sqlite") {
        Some(path) => path,
        None => match basedirs.place_data_file("data.sqlite") {
            Ok(path) => path,
            Err(err) => { eprintln!("error placing config: {}", err); return; }
        }
    };
    let db = match SqlConnection::open(&path) {
        Ok(ok) => ok,
        Err(err) => {
            eprintln!("Failed to open database");
            eprintln!("{}", err);
            return;
        }
    };
    let db = Rc::new(db);
    db.execute("CREATE TABLE IF NOT EXISTS data (
                    key     TEXT NOT NULL UNIQUE,
                    value   TEXT NOT NULL
                )", &[])
        .expect("Couldn't create SQLite table");
    db.execute("CREATE TABLE IF NOT EXISTS servers (
                    ip      TEXT NOT NULL PRIMARY KEY,
                    name    TEXT NOT NULL,
                    hash    BLOB NOT NULL,
                    token   TEXT
                )", &[])
        .expect("Couldn't create SQLite table");

    let nick = {
        let mut stmt = db.prepare("SELECT value FROM data WHERE key = 'nick'").unwrap();
        let mut rows = stmt.query(&[]).unwrap();

        if let Some(row) = rows.next() {
            row.unwrap().get::<_, String>(0)
        } else {
            #[cfg(unix)]
            { env::var("USER").unwrap_or_else(|_| String::from("unknown")) }
            #[cfg(windows)]
            { env::var("USERNAME").unwrap_or_else(|_| String::from("unknown")) }
            #[cfg(not(any(unix, windows)))]
            { String::from("unknown") }
        }
    };

    if let Err(err) = gtk::init() {
        eprintln!("gtk error: {}", err);
        return;
    }

    let connections = Connections::new(&db, nick);

    let window = Window::new(WindowType::Toplevel);
    window.set_title("Synac GTK+ client");
    window.set_default_size(1000, 700);

    let stack = Stack::new();
    let main = GtkBox::new(Orientation::Horizontal, 10);

    let servers_wrapper = GtkBox::new(Orientation::Vertical, 0);

    let user_name = Label::new(&**connections.nick.read().unwrap());
    user_name.set_property_margin(10);
    servers_wrapper.add(&user_name);

    servers_wrapper.add(&Separator::new(Orientation::Vertical));

    // Are used in the future, but needed right now
    let server_name = Label::new("");
    let channels = GtkBox::new(Orientation::Vertical, 2);
    let channel_name = Label::new("");
    let messages = GtkBox::new(Orientation::Vertical, 2);
    // end

    let servers = GtkBox::new(Orientation::Vertical, 2);
    render_servers(&connections, &db, &servers, &server_name, &channels, &channel_name, &messages, &window);
    servers_wrapper.add(&servers);

    let add = Button::new_with_label("Add...");
    add.set_valign(Align::End);
    add.set_vexpand(true);

    servers_wrapper.add(&add);

    main.add(&servers_wrapper);

    main.add(&Separator::new(Orientation::Horizontal));

    let channels_wrapper = GtkBox::new(Orientation::Vertical, 0);

    server_name.set_property_margin(10);
    channels_wrapper.add(&server_name);

    channels_wrapper.add(&Separator::new(Orientation::Vertical));

    channels_wrapper.add(&channels);

    main.add(&channels_wrapper);

    main.add(&Separator::new(Orientation::Horizontal));

    let content = GtkBox::new(Orientation::Vertical, 2);

    channel_name.set_property_margin(10);
    content.add(&channel_name);

    content.add(&Separator::new(Orientation::Vertical));

    messages.set_vexpand(true);
    messages.set_valign(Align::Center);
    let scroll = ScrolledWindow::new(None, None);
    scroll.add(&messages);

    scroll.get_vadjustment().unwrap().connect_changed(move |vadjustment| {
        let upper = vadjustment.get_upper() - vadjustment.get_page_size();
        if vadjustment.get_value() + 100.0 >= upper {
            vadjustment.set_value(upper);
        }
    });
    let connections_clone = Arc::clone(&connections);
    scroll.connect_edge_reached(move |_, pos| {
        if pos != PositionType::Top {
            return;
        }
        if let Some(addr) = *connections_clone.current_server.lock().unwrap() {
            connections_clone.execute(addr, |result| {
                if let Ok(synac) = result {
                    if let Some(channel) = synac.current_channel {
                        if let Err(err) = synac.session.send(&Packet::MessageList(common::MessageList {
                            after: None,
                            before: synac.messages.get(channel).first().map(|msg| msg.id),
                            channel: channel,
                            limit: common::LIMIT_BULK
                        })) {
                            eprintln!("error sending packet: {}", err);
                        }
                    }
                }
            });
        }
    });
    content.add(&scroll);

    let input = Entry::new();
    input.set_hexpand(true);
    input.set_placeholder_text("Send a message");

    let channel_name_clone = channel_name.clone();
    let channels_clone = channels.clone();
    let connections_clone = Arc::clone(&connections);
    let db_clone = Rc::clone(&db);
    let messages_clone = messages.clone();
    let server_name_clone = server_name.clone();
    let window_clone = window.clone();
    input.connect_activate(move |input| {
        let text = input.get_text().unwrap_or_default();
        if text.is_empty() {
            return;
        }
        input.set_sensitive(false);
        if let Some(addr) = *connections_clone.current_server.lock().unwrap() {
            connections_clone.execute(addr, |result| {
                if result.is_err() {
                    return;
                }
                let synac = result.unwrap();
                if synac.current_channel.is_none() {
                    return;
                }
                let channel = synac.current_channel.unwrap();
                if let Err(err) = synac.session.send(&Packet::MessageCreate(common::MessageCreate {
                    channel: channel,
                    text: text.into_bytes()
                })) {
                    if let Ok(io_err) = err.downcast::<IoError>() {
                        if io_err.kind() != IoErrorKind::BrokenPipe {
                            return;
                        }
                    }

                    let mut stmt = db_clone.prepare("SELECT hash, token FROM servers WHERE ip = ?").unwrap();
                    let mut rows = stmt.query(&[&addr.to_string()]).unwrap();

                    if let Some(row) = rows.next() {
                        let row = row.unwrap();

                        let hash = row.get(0);
                        let token = row.get(1);

                        connect(addr, &connections_clone, &db_clone, hash, token, &server_name_clone,
                                &channel_name_clone, &channels_clone, &messages_clone, &window_clone);
                    }
                }
            });
        }
        input.set_text("");
        input.set_sensitive(true);
    });

    content.add(&input);

    main.add(&content);
    stack.add(&main);

    let add_server = GtkBox::new(Orientation::Vertical, 2);

    let name = Entry::new();
    name.set_placeholder_text("Name");
    add_server.add(&name);
    add_server.add(&Label::new("The server name. This can be anything you want it to."));

    let server = Entry::new();
    server.set_placeholder_text("Server IP");
    add_server.add(&server);
    add_server.add(&Label::new(&*format!("The server IP address. The default port is {}.", common::DEFAULT_PORT)));

    let hash = Entry::new();
    hash.set_placeholder_text("Server's certificate hash");
    add_server.add(&hash);
    add_server.add(&Label::new("The server's certificate public key hash.\n\
                               This is to verify nobody is snooping on your connection"));

    let add_server_controls = GtkBox::new(Orientation::Horizontal, 2);

    let add_server_cancel = Button::new_with_label("Cancel");
    let stack_clone = stack.clone();
    let main_clone  = main.clone();
    add_server_cancel.connect_clicked(move |_| {
        stack_clone.set_visible_child(&main_clone);
    });
    add_server_controls.add(&add_server_cancel);

    let add_server_ok = Button::new_with_label("Ok");

    let channel_name_clone = channel_name.clone();
    let channels_clone = channels.clone();
    let connections_clone = Arc::clone(&connections);
    let db_clone = Rc::clone(&db);
    let main_clone  = main.clone();
    let messages_clone = messages.clone();
    let server_name_clone = server_name.clone();
    let servers_clone = servers.clone();
    let stack_clone = stack.clone();
    let window_clone = window.clone();
    add_server_ok.connect_clicked(move |_| {
        let name_text   = name.get_text().unwrap_or_default();
        let server_text = server.get_text().unwrap_or_default();
        let hash_text   = hash.get_text().unwrap_or_default();

        let addr = match connections::parse_addr(&server_text) {
            Some(addr) => addr,
            None => return
        };

        name.set_text("");
        server.set_text("");
        hash.set_text("");

        stack_clone.set_visible_child(&main_clone);

        db_clone.execute(
            "INSERT INTO servers (name, ip, hash) VALUES (?, ?, ?)",
            &[&name_text, &addr.to_string(), &hash_text]
        ).unwrap();
        render_servers(&connections_clone, &db_clone, &servers_clone,
                       &server_name_clone, &channels_clone, &channel_name_clone,
                       &messages_clone, &window_clone);
    });
    add_server_controls.add(&add_server_ok);

    add_server.add(&add_server_controls);
    stack.add(&add_server);

    let stack_clone = stack.clone();
    let add_server = add_server.clone();
    add.connect_clicked(move |_| {
        stack_clone.set_visible_child(&add_server);
    });

    window.add(&stack);

    // Load CSS
    let screen = Screen::get_default();
    match screen {
        None => eprintln!("error: no default screen"),
        Some(screen) => {
            let css = CssProvider::new();
            let result: Result<(), Error> = if let Some(file) = basedirs.find_config_file("style.css") {
                if let Some(s) = file.to_str() {
                    css.load_from_path(s).map_err(Error::from)
                } else {
                    Err(UnicodePathError.into())
                }
            } else {
                css.load_from_data(include_bytes!("style.css")).map_err(Error::from)
            };
            if let Err(err) = result {
                alert(&window, MessageType::Error, &err.to_string());
            }
            StyleContext::add_provider_for_screen(&screen, &css, STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
    }

    window.show_all();
    window.connect_delete_event(|_, _| {
        gtk::main_quit();
        Inhibit(false)
    });

    let channel_name_clone = channel_name.clone();
    let channels_clone = channels.clone();
    let messages_clone = messages.clone();
    gtk::idle_add(move || {
        let mut channels = false;
        let mut messages = false;
        let mut addr = None;

        if let Err(err) = connections.try_read(|synac, packet| {
            println!("received {:?}", packet);
            if *connections.current_server.lock().unwrap() != Some(synac.addr) {
                return;
            }
            addr = Some(synac.addr);
            match packet {
                Packet::ChannelReceive(_)       => channels = true,
                Packet::ChannelDeleteReceive(_) => channels = true,
                Packet::MessageReceive(_)       => messages = true,
                Packet::MessageDeleteReceive(_)       => messages = true,
                _ => {}
            }
        }) {
            eprintln!("receive error: {}", err);
            return Continue(true);
        }

        if let Some(addr) = addr {
            if channels {
                render_channels(Some((&connections, addr)), &channels_clone, &channel_name_clone, &messages_clone);
            } else if messages {
                render_messages(Some((&connections, addr)), &messages_clone);
            }
        }

        Continue(true)
    });
    gtk::main();
}
fn alert(window: &Window, kind: MessageType, message: &str) {
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
fn connect(addr: SocketAddr, connections: &Arc<Connections>, db: &Rc<SqlConnection>,
           hash: String, token: Option<String>, server_name: &Label, channel_name: &Label,
           channels: &GtkBox, messages: &GtkBox, window: &Window)
    -> Option<Error>
{
    let result = connections.connect(addr, hash, token, || {
        let dialog = Dialog::new_with_buttons(
            Some("Synac: Password dialog"),
            Some(window),
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
        Some((text, Rc::clone(&db)))
    });
    match result {
        Ok(session) => {
            let synac = Synac::new(addr, session);
            connections.insert(addr, synac);
            connections.set_current(Some(addr));
            render_channels(Some((&connections, addr)), channels, &channel_name, &messages);
            None
        },
        Err(err)  => {
            alert(&window, MessageType::Error, &format!("connection error: {}", err));
            server_name.set_text("");
            connections.set_current(None);
            render_channels(None, &channels, &channel_name, &messages);
            Some(err)
        }
    }
}
fn render_servers(connections: &Arc<Connections>, db: &Rc<SqlConnection>, servers: &GtkBox,
                  server_name: &Label, channels: &GtkBox, channel_name: &Label,
                  messages: &GtkBox, window: &Window) {
    for child in servers.get_children() {
        servers.remove(&child);
    }
    let mut stmt = db.prepare("SELECT ip, name, hash, token FROM servers ORDER BY name").unwrap();
    let mut rows = stmt.query(&[]).unwrap();

    while let Some(row) = rows.next() {
        let row = row.unwrap();
        let addr:   String = row.get(0);
        let name: String = row.get(1);
        let hash: String = row.get(2);
        let token: Option<String> = row.get(3);

        let ip_parsed = connections::parse_addr(&addr);

        let button = Button::new_with_label(&name);
        let channel_name_clone = channel_name.clone();
        let channels_clone = channels.clone();
        let connections_clone = Arc::clone(connections);
        let db_clone = Rc::clone(db);
        let messages_clone = messages.clone();
        let server_name_clone = server_name.clone();
        let window_clone = window.clone();
        button.connect_clicked(move |_| {
            let addr = match ip_parsed {
                Some(addr) => addr,
                None => {
                    alert(&window_clone, MessageType::Error, "Failed to parse IP address. Format: <ip[:port]>");
                    return;
                }
            };
            println!("Server with IP {} was clicked", addr);
            server_name_clone.set_text(&name);
            let mut ok = false;
            connections_clone.execute(addr, |result| {
                ok = result.is_ok();
            });
            if ok {
                connections_clone.set_current(Some(addr));
                render_channels(Some((&connections_clone, addr)), &channels_clone, &channel_name_clone,
                                &messages_clone);
            } else {
                connect(addr, &connections_clone, &db_clone, hash.clone(), token.clone(), &server_name_clone,
                        &channel_name_clone, &channels_clone, &messages_clone, &window_clone);
            }
        });

        let channel_name_clone = channel_name.clone();
        let channels_clone = channels.clone();
        let connections_clone = Arc::clone(connections);
        let db_clone = Rc::clone(db);
        let messages_clone = messages.clone();
        let server_name_clone = server_name.clone();
        let servers_clone = servers.clone();
        let window_clone = window.clone();
        button.connect_button_press_event(move |_, event| {
            if event.get_button() == 3 {
                let menu = Menu::new();

                let forget = MenuItem::new_with_label("Forget server");

                let channel_name_clone = channel_name_clone.clone();
                let channels_clone = channels_clone.clone();
                let connections_clone = Arc::clone(&connections_clone);
                let db_clone = Rc::clone(&db_clone);
                let ip_clone = addr.clone();
                let messages_clone = messages_clone.clone();
                let server_name_clone = server_name_clone.clone();
                let servers_clone = servers_clone.clone();
                let window_clone = window_clone.clone();
                forget.connect_activate(move |_| {
                    db_clone.execute("DELETE FROM servers WHERE ip = ?", &[&ip_clone.to_string()]).unwrap();
                    render_servers(&connections_clone, &db_clone, &servers_clone,
                                   &server_name_clone, &channels_clone, &channel_name_clone,
                                   &messages_clone, &window_clone);
                });
                menu.add(&forget);

                menu.show_all();
                menu.popup_at_pointer(Some(&**event));
            }
            Inhibit(false)
        });
        servers.add(&button);
    }
    servers.show_all();
    servers.queue_draw();
}
fn render_channels(connection: Option<(&Arc<Connections>, SocketAddr)>, channels: &GtkBox, channel_name: &Label,
                   messages: &GtkBox) {
    for child in channels.get_children() {
        channels.remove(&child);
    }
    if let Some((connection, addr)) = connection {
        connection.execute(addr, |result| {
            if let Ok(server) = result {
                let mut channel_list: Vec<_> = server.state.channels.values().collect();
                channel_list.sort_by_key(|channel| &channel.name);
                for channel in channel_list {
                    let mut name = String::with_capacity(channel.name.len() + 1);
                    name.push('#');
                    name.push_str(&channel.name);

                    let button = Button::new_with_label(&name);

                    let channel_id = channel.id;
                    let channel_name_clone = channel_name.clone();
                    let connection_clone = connection.clone();
                    let messages_clone = messages.clone();
                    button.connect_clicked(move |_| {
                        connection_clone.execute(addr, |result| {
                            if let Ok(synac) = result {
                                synac.current_channel = Some(channel_id);
                                channel_name_clone.set_text(&name);
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
                        render_messages(Some((&connection_clone, addr)), &messages_clone);
                    });
                    channels.add(&button);
                }
            }
        });
    } else {
        channel_name.set_text("");
        render_messages(None, &messages);
    }

    channels.show_all();
    channels.queue_draw();
}
fn render_messages(connection: Option<(&Arc<Connections>, SocketAddr)>, messages: &GtkBox) {
    for child in messages.get_children() {
        messages.remove(&child);
    }
    if let Some(connection) = connection {
        connection.0.execute(connection.1, |result| {
            if let Ok(synac) = result {
                if let Some(channel) = synac.current_channel {
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
                            time.push_str(")");
                        }
                        let time = Label::new(&*time);
                        time.set_xalign(0.0);
                        authorbox.add(&time);

                        msgbox.add(&authorbox);

                        let text = Label::new(&*String::from_utf8_lossy(&msg.text));
                        text.set_xalign(0.0);
                        msgbox.add(&text);

                        messages.add(&msgbox);

                        messages.add(&Separator::new(Orientation::Vertical));
                    }
                }
            }
        });
    }
    messages.show_all();
    messages.queue_draw();
}
