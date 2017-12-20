#[macro_use] extern crate failure;
extern crate chrono;
extern crate gdk;
extern crate gtk;
extern crate rusqlite;
extern crate synac;
extern crate xdg;

mod connections;
mod messages;
mod typing;

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
    EventBox,
    InputPurpose,
    Label,
    Menu,
    MenuItem,
    MessageDialog,
    MessageType,
    Orientation,
    PositionType,
    ResponseType,
    Revealer,
    RevealerTransitionType,
    ScrolledWindow,
    Separator,
    Stack,
    StyleContext,
    STYLE_PROVIDER_PRIORITY_APPLICATION,
    Window,
    WindowType
};
use connections::Connections;
use rusqlite::Connection as SqlConnection;
use std::cell::RefCell;
use std::env;
use std::io::{Error as IoError, ErrorKind as IoErrorKind};
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use synac::common::{self, Packet};
use xdg::BaseDirectories;

#[derive(Debug, Fail)]
#[fail(display = "sadly GTK+ doesn't support unicode paths")]
struct UnicodePathError;

struct App {
    connections: Arc<Connections>,
    db: Rc<SqlConnection>,

    channel_name: Label,
    channels: GtkBox,
    message_edit: Revealer,
    message_edit_id: RefCell<Option<usize>>,
    message_edit_input: Entry,
    messages: GtkBox,
    messages_scroll: ScrolledWindow,
    server_name: Label,
    servers: GtkBox,
    stack: Stack,
    stack_add_server: GtkBox,
    stack_main: GtkBox,
    typing: Label,
    window: Window
}

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

    let window = Window::new(WindowType::Toplevel);
    window.set_title("Synac GTK+ client");
    window.set_default_size(1000, 700);

    let app = Rc::new(App {
        channel_name: Label::new(""),
        channels: GtkBox::new(Orientation::Vertical, 2),
        connections: Connections::new(&db, nick),
        db: Rc::new(db),
        message_edit: Revealer::new(),
        message_edit_id: RefCell::new(None),
        message_edit_input: Entry::new(),
        messages: GtkBox::new(Orientation::Vertical, 2),
        messages_scroll: ScrolledWindow::new(None, None),
        server_name: Label::new(""),
        servers: GtkBox::new(Orientation::Vertical, 2),
        stack: Stack::new(),
        stack_add_server: GtkBox::new(Orientation::Vertical, 2),
        stack_main: GtkBox::new(Orientation::Horizontal, 10),
        typing: Label::new(""),
        window: window.clone()
    });

    let servers_wrapper = GtkBox::new(Orientation::Vertical, 0);

    let user_name = Label::new(&**app.connections.nick.read().unwrap());
    user_name.set_property_margin(10);
    servers_wrapper.add(&user_name);

    servers_wrapper.add(&Separator::new(Orientation::Vertical));

    render_servers(&app);
    servers_wrapper.add(&app.servers);

    let add = Button::new_with_label("Add...");
    add.set_valign(Align::End);
    add.set_vexpand(true);

    servers_wrapper.add(&add);

    app.stack_main.add(&servers_wrapper);

    app.stack_main.add(&Separator::new(Orientation::Horizontal));

    let channels_wrapper = GtkBox::new(Orientation::Vertical, 0);

    app.server_name.set_property_margin(10);
    channels_wrapper.add(&app.server_name);

    channels_wrapper.add(&Separator::new(Orientation::Vertical));

    channels_wrapper.add(&app.channels);

    app.stack_main.add(&channels_wrapper);

    app.stack_main.add(&Separator::new(Orientation::Horizontal));

    let content = GtkBox::new(Orientation::Vertical, 2);

    app.channel_name.set_property_margin(10);
    content.add(&app.channel_name);

    content.add(&Separator::new(Orientation::Vertical));

    app.messages.set_vexpand(true);
    app.messages_scroll.add(&app.messages);

    app.messages_scroll.get_vadjustment().unwrap().connect_changed(move |vadjustment| {
        let upper = vadjustment.get_upper() - vadjustment.get_page_size();
        if vadjustment.get_value() + 100.0 >= upper {
            vadjustment.set_value(upper);
        }
    });
    let app_clone = Rc::clone(&app);
    app.messages_scroll.connect_edge_reached(move |_, pos| {
        if pos != PositionType::Top {
            return;
        }
        if let Some(addr) = *app_clone.connections.current_server.lock().unwrap() {
            app_clone.connections.execute(addr, |result| {
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
    content.add(&app.messages_scroll);

    app.message_edit.set_transition_type(RevealerTransitionType::SlideUp);

    let message_edit = GtkBox::new(Orientation::Vertical, 2);

    message_edit.add(&Label::new("Edit message"));

    let app_clone = Rc::clone(&app);
    app.message_edit_input.connect_activate(move |input| {
        let text = input.get_text().unwrap_or_default();
        if text.is_empty() {
            return;
        }
        input.set_sensitive(false);
        if let Some(addr) = *app_clone.connections.current_server.lock().unwrap() {
            app_clone.connections.execute(addr, |result| {
                if result.is_err() {
                    return;
                }
                let synac = result.unwrap();

                if let Err(err) = synac.session.send(&Packet::MessageUpdate(common::MessageUpdate {
                    id: app_clone.message_edit_id.borrow().expect("wait how is this variable not set"),
                    text: text.into_bytes()
                })) {
                    eprintln!("failed to send packet: {}", err);
                }
            });
        }
        input.set_sensitive(true);
        app_clone.message_edit.set_reveal_child(false);
    });

    message_edit.add(&app.message_edit_input);

    let message_edit_cancel = Button::new_with_label("Cancel");
    let app_clone = Rc::clone(&app);
    message_edit_cancel.connect_clicked(move |_| {
        app_clone.message_edit.set_reveal_child(false);
    });
    message_edit.add(&message_edit_cancel);

    app.message_edit.add(&message_edit);
    content.add(&app.message_edit);

    let input = Entry::new();
    input.set_hexpand(true);
    input.set_placeholder_text("Send a message");

    let typing_duration = Duration::from_secs(common::TYPING_TIMEOUT as u64 / 2); // TODO: const fn
    let typing_last = RefCell::new(Instant::now());

    let app_clone = Rc::clone(&app);
    input.connect_property_text_notify(move |_| {
        let mut typing_last = typing_last.borrow_mut();
        if typing_last.elapsed() < typing_duration {
            return;
        }
        *typing_last = Instant::now();

        if let Some(addr) = *app_clone.connections.current_server.lock().unwrap() {
            app_clone.connections.execute(addr, |result| {
                if let Ok(synac) = result {
                    if let Some(channel) = synac.current_channel {
                        if let Err(err) = synac.session.send(&Packet::Typing(common::Typing {
                            channel: channel
                        })) {
                            eprintln!("failed to send packet: {}", err);
                        }
                    }
                }
            });
        }
    });
    let app_clone = Rc::clone(&app);
    input.connect_activate(move |input| {
        let text = input.get_text().unwrap_or_default();
        if text.is_empty() {
            return;
        }
        input.set_sensitive(false);
        if let Some(addr) = *app_clone.connections.current_server.lock().unwrap() {
            app_clone.connections.execute(addr, |result| {
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

                    let mut stmt = app_clone.db.prepare("SELECT hash, token FROM servers WHERE ip = ?").unwrap();
                    let mut rows = stmt.query(&[&addr.to_string()]).unwrap();

                    if let Some(row) = rows.next() {
                        let row = row.unwrap();

                        let hash = row.get(0);
                        let token = row.get(1);

                        connect(&app_clone, addr, hash, token);
                    }
                }
            });
        }
        input.set_text("");
        input.set_sensitive(true);
    });

    content.add(&input);

    app.typing.set_xalign(0.0);
    content.add(&app.typing);

    app.stack_main.add(&content);

    app.stack.add(&app.stack_main);

    let name = Entry::new();
    name.set_placeholder_text("Name");
    app.stack_add_server.add(&name);
    app.stack_add_server.add(&Label::new("The server name. This can be anything you want it to."));

    let server = Entry::new();
    server.set_placeholder_text("Server IP");
    app.stack_add_server.add(&server);
    app.stack_add_server.add(&Label::new(&*format!("The server IP address. The default port is {}.", common::DEFAULT_PORT)));

    let hash = Entry::new();
    hash.set_placeholder_text("Server's certificate hash");
    app.stack_add_server.add(&hash);
    app.stack_add_server.add(&Label::new("The server's certificate public key hash.\n\
                               This is to verify nobody is snooping on your connection"));

    let add_server_controls = GtkBox::new(Orientation::Horizontal, 2);

    let add_server_cancel = Button::new_with_label("Cancel");
    let app_clone = Rc::clone(&app);
    add_server_cancel.connect_clicked(move |_| {
        app_clone.stack.set_visible_child(&app_clone.stack_main);
    });
    add_server_controls.add(&add_server_cancel);

    let add_server_ok = Button::new_with_label("Ok");

    let app_clone = Rc::clone(&app);
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

        app_clone.stack.set_visible_child(&app_clone.stack_main);

        app_clone.db.execute(
            "INSERT INTO servers (name, ip, hash) VALUES (?, ?, ?)",
            &[&name_text, &addr.to_string(), &hash_text]
        ).unwrap();
        render_servers(&app_clone);
    });
    add_server_controls.add(&add_server_ok);

    app.stack_add_server.add(&add_server_controls);
    app.stack.add(&app.stack_add_server);

    let app_clone = Rc::clone(&app);
    add.connect_clicked(move |_| {
        app_clone.stack.set_visible_child(&app_clone.stack_add_server);
    });

    window.add(&app.stack);

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

    gtk::idle_add(move || {
        let mut channels = false;
        let mut messages = false;
        let mut addr = None;

        let current_server = *app.connections.current_server.lock().unwrap();

        if let Err(err) = app.connections.try_read(|synac, packet| {
            println!("received {:?}", packet);
            if current_server != Some(synac.addr) {
                return;
            }
            addr = Some(synac.addr);
            match packet {
                Packet::ChannelReceive(_)       => channels = true,
                Packet::ChannelDeleteReceive(_) => channels = true,
                Packet::MessageReceive(_)       => messages = true,
                Packet::MessageDeleteReceive(_) => messages = true,
                _ => {}
            }
        }) {
            eprintln!("receive error: {}", err);
            return Continue(true);
        }

        if let Some(addr) = current_server {
            app.connections.execute(addr, |result| {
                if let Ok(synac) = result {
                    if let Some(typing) = synac.typing.check(synac.current_channel, &synac.state) {
                        app.typing.set_text(&typing);
                    }
                }
            });
        }

        if let Some(addr) = addr {
            if channels {
                render_channels(Some(addr), &app);
            } else if messages {
                render_messages(Some(addr), &app);
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
fn connect(app: &Rc<App>, addr: SocketAddr, hash: String, token: Option<String>)
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
        Err(err)  => {
            app.connections.set_current(None);
            app.server_name.set_text("");
            alert(&app.window, MessageType::Error, &format!("connection error: {}", err));
            app.message_edit.set_reveal_child(false);
            render_channels(None, app);
            Some(err)
        }
    }
}
fn render_servers(app: &Rc<App>) {
    for child in app.servers.get_children() {
        app.servers.remove(&child);
    }
    let mut stmt = app.db.prepare("SELECT ip, name, hash, token FROM servers ORDER BY name").unwrap();
    let mut rows = stmt.query(&[]).unwrap();

    while let Some(row) = rows.next() {
        let row = row.unwrap();
        let addr:   String = row.get(0);
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

                let forget = MenuItem::new_with_label("Forget server");

                let app_clone = Rc::clone(&app_clone);
                let addr_clone = addr.clone();
                forget.connect_activate(move |_| {
                    app_clone.db.execute("DELETE FROM servers WHERE ip = ?", &[&addr_clone]).unwrap();
                    render_servers(&app_clone);
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
fn render_channels(addr: Option<SocketAddr>, app: &Rc<App>) {
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
fn render_messages(addr: Option<SocketAddr>, app: &Rc<App>) {
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
                text.set_xalign(0.0);

                let event = EventBox::new();
                event.add(&text);

                let app_clone = Rc::clone(&app);
                let msg_id = msg.id;
                event.connect_button_press_event(move |_, event| {
                    if event.get_button() == 3 {
                        let menu = Menu::new();

                        let edit = MenuItem::new_with_label("Edit message");

                        let app_clone = Rc::clone(&app_clone);
                        let string = string.clone();
                        edit.connect_activate(move |_| {
                            *app_clone.message_edit_id.borrow_mut() = Some(msg_id);
                            app_clone.message_edit_input.set_text(&string);
                            app_clone.message_edit.set_reveal_child(true);
                        });

                        menu.add(&edit);

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
