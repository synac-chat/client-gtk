#[macro_use] extern crate failure;
extern crate chrono;
extern crate gdk;
extern crate gtk;
extern crate notify_rust;
extern crate pango;
extern crate pulldown_cmark;
extern crate rusqlite;
extern crate synac;
extern crate xdg;

mod connections;
mod functions;
mod messages;
mod parser;
mod typing;

use gtk::{
    Align,
    Box as GtkBox,
    Button,
    ButtonsType,
    CheckButton,
    CssProvider,
    Dialog,
    DialogFlags,
    Entry,
    EventBox,
    IconSize,
    InputPurpose,
    Label,
    Menu,
    MenuItem,
    MessageDialog,
    MessageType,
    Orientation,
    PolicyType,
    PositionType,
    RadioButton,
    ResponseType,
    Revealer,
    RevealerTransitionType,
    ScrolledWindow,
    Separator,
    SeparatorMenuItem,
    Stack,
    StackTransitionType,
    StyleContext,
    STYLE_PROVIDER_PRIORITY_APPLICATION,
    Window,
    WindowType
};
use connections::{Connections, Synac};
use failure::Error;
use functions::*;
use gdk::Screen;
use gtk::prelude::*;
use notify_rust::Notification;
use pango::WrapMode;
use rusqlite::Connection as SqlConnection;
use std::cell::RefCell;
use std::env;
use std::fmt::Write;
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

struct EditChannel {
    container: GtkBox,
    edit: RefCell<Option<usize>>,

    name: Entry,
    mode_bots: GtkBox,
    mode_users: GtkBox
}
struct EditServer {
    container: GtkBox,

    name: Entry,
    server: Entry,
    hash: Entry
}
struct EditUser {
    container: GtkBox,
    user: RefCell<Option<usize>>,

    radio_none: RadioButton,
    radio_some: RadioButton,
    mode: GtkBox
}
struct App {
    connections: Arc<Connections>,
    db: Rc<SqlConnection>,

    channel_add: Revealer,
    channel_name: Label,
    channels: GtkBox,
    channels_priv: GtkBox,
    message_edit: Revealer,
    message_edit_id: RefCell<Option<usize>>,
    message_edit_input: Entry,
    message_input: Revealer,
    messages: GtkBox,
    messages_noread: Revealer,
    messages_scroll: ScrolledWindow,
    server_name: Label,
    servers: GtkBox,
    stack: Stack,
    stack_edit_channel: EditChannel,
    stack_edit_server: EditServer,
    stack_edit_user: EditUser,
    stack_main: GtkBox,
    typing: Label,
    user_stack: Stack,
    user_stack_edit: Entry,
    user_stack_text: EventBox,
    users: GtkBox,
    users_revealer: Revealer,
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
                    key     TEXT NOT NULL PRIMARY KEY UNIQUE,
                    value   TEXT NOT NULL
                )", &[])
        .expect("Couldn't create SQLite table");
    db.execute("CREATE TABLE IF NOT EXISTS servers (
                    ip      TEXT NOT NULL PRIMARY KEY UNIQUE,
                    name    TEXT NOT NULL,
                    hash    BLOB NOT NULL,
                    token   TEXT
                )", &[])
        .expect("Couldn't create SQLite table");
    db.execute("CREATE TABLE IF NOT EXISTS muted (
                    channel INTEGER NOT NULL,
                    server  TEXT    NOT NULL
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
    window.set_default_size(1152, 648);
    window.set_position(gtk::WindowPosition::Center);

    let radio_none = RadioButton::new_with_label("Inherit channel's mode");
    let radio_some = RadioButton::new_with_label_from_widget(&radio_none, "Use custom mode:");

    let app = Rc::new(App {
        channel_add: Revealer::new(),
        channel_name: Label::new(""),
        channels: GtkBox::new(Orientation::Vertical, 2),
        channels_priv: GtkBox::new(Orientation::Vertical, 2),
        connections: Connections::new(&db, nick),
        db: Rc::new(db),
        message_edit: Revealer::new(),
        message_edit_id: RefCell::new(None),
        message_edit_input: Entry::new(),
        message_input: Revealer::new(),
        messages: GtkBox::new(Orientation::Vertical, 3),
        messages_noread: Revealer::new(),
        messages_scroll: ScrolledWindow::new(None, None),
        server_name: Label::new(""),
        servers: GtkBox::new(Orientation::Vertical, 2),
        stack: Stack::new(),
        stack_edit_channel: EditChannel {
            container: GtkBox::new(Orientation::Vertical, 2),
            edit: RefCell::new(None),

            name: Entry::new(),
            mode_bots: GtkBox::new(Orientation::Vertical, 2),
            mode_users: GtkBox::new(Orientation::Vertical, 2)
        },
        stack_edit_server: EditServer {
            container: GtkBox::new(Orientation::Vertical, 2),

            name: Entry::new(),
            server: Entry::new(),
            hash: Entry::new()
        },
        stack_edit_user: EditUser {
            container: GtkBox::new(Orientation::Vertical, 2),
            user: RefCell::new(None),

            radio_none: radio_none,
            radio_some: radio_some,
            mode: GtkBox::new(Orientation::Vertical, 2)
        },
        stack_main: GtkBox::new(Orientation::Horizontal, 10),
        user_stack: Stack::new(),
        user_stack_edit: Entry::new(),
        user_stack_text: EventBox::new(),
        users: GtkBox::new(Orientation::Vertical, 2),
        users_revealer: Revealer::new(),
        typing: Label::new(""),
        window: window
    });

    app.channel_add.set_transition_type(RevealerTransitionType::SlideUp);
    app.message_edit.set_transition_type(RevealerTransitionType::SlideUp);
    app.message_input.set_transition_type(RevealerTransitionType::SlideUp);
    app.messages_noread.set_transition_type(RevealerTransitionType::SlideDown);
    app.stack.set_transition_type(StackTransitionType::SlideLeftRight);
    app.user_stack.set_transition_type(StackTransitionType::Crossfade);
    app.users_revealer.set_transition_type(RevealerTransitionType::SlideLeft);

    app.stack.add(&app.stack_main);
    app.stack.add(&app.stack_edit_server.container);
    app.stack.add(&app.stack_edit_channel.container);
    app.stack.add(&app.stack_edit_user.container);

    app.user_stack.add(&app.user_stack_text);
    app.user_stack.add(&app.user_stack_edit);

    let user_name = Label::new(&**app.connections.nick.read().unwrap());
    add_class(&user_name, "bold");

    app.user_stack_edit.set_alignment(0.5);

    let app_clone = Rc::clone(&app);
    let user_name_clone = user_name.clone();
    app.user_stack_edit.connect_activate(move |input| {
        let text = input.get_text().unwrap_or_default();
        let old = app_clone.connections.nick.read().unwrap();
        app_clone.user_stack.set_visible_child(&app_clone.user_stack_text);
        if text.is_empty() || text == *old {
            return;
        }
        user_name_clone.set_text(&text);

        drop(old);

        app_clone.connections.foreach(|synac| {
            let result = synac.session.write(&Packet::LoginUpdate(common::LoginUpdate {
                name: Some(text.clone()),
                password_current: None,
                password_new: None,
                reset_token: false
            }));
            if let Err(err) = result {
                let string = format!("failed to update server {}: {}", synac.addr, err.to_string());
                alert(&app_clone.window, MessageType::Warning, &string);
            }
        });

        app_clone.db.execute("REPLACE INTO data (key, value) VALUES ('nick', ?)", &[&text]).unwrap();

        *app_clone.connections.nick.write().unwrap() = text;
    });
    let app_clone = Rc::clone(&app);
    app.user_stack_edit.connect_focus_out_event(move |_, _| {
        app_clone.user_stack.set_visible_child(&app_clone.user_stack_text);
        Inhibit(false)
    });

    let servers_wrapper = GtkBox::new(Orientation::Vertical, 0);

    user_name.set_property_margin(10);
    app.user_stack_text.add(&user_name);

    let app_clone = Rc::clone(&app);
    app.user_stack_text.connect_button_press_event(move |_, event| {
        if event.get_button() == 1 {
            app_clone.user_stack_edit.set_text(&app_clone.connections.nick.read().unwrap());
            app_clone.user_stack_edit.grab_focus();
            app_clone.user_stack.set_visible_child(&app_clone.user_stack_edit);
        }
        Inhibit(false)
    });
    servers_wrapper.add(&app.user_stack);

    servers_wrapper.add(&Separator::new(Orientation::Vertical));

    render_servers(&app);
    let scroll = ScrolledWindow::new(None, None);
    scroll.set_vexpand(true);
    scroll.add(&app.servers);
    servers_wrapper.add(&scroll);

    let add = Button::new_with_mnemonic("Add _Server");
    add_class(&add, "add");
    add.set_valign(Align::End);
    add.set_vexpand(true);

    let app_clone = Rc::clone(&app);
    add.connect_clicked(move |_| {
        app_clone.stack_edit_server.name.set_text("");
        app_clone.stack_edit_server.server.set_text("");
        app_clone.stack_edit_server.server.set_sensitive(true);
        app_clone.stack_edit_server.hash.set_text("");

        app_clone.stack.set_visible_child(&app_clone.stack_edit_server.container);
    });

    servers_wrapper.add(&add);

    app.stack_main.add(&servers_wrapper);

    app.stack_main.add(&Separator::new(Orientation::Horizontal));

    let channels_wrapper = GtkBox::new(Orientation::Vertical, 0);

    add_class(&app.server_name, "bold");
    app.server_name.set_property_margin(10);
    channels_wrapper.add(&app.server_name);

    channels_wrapper.add(&Separator::new(Orientation::Vertical));

    let scroll = ScrolledWindow::new(None, None);
    scroll.set_vexpand(true);
    scroll.add(&app.channels);
    channels_wrapper.add(&scroll);

    channels_wrapper.add(&Separator::new(Orientation::Vertical));
    channels_wrapper.add(&Label::new("Private channels:"));

    let scroll = ScrolledWindow::new(None, None);
    scroll.set_vexpand(true);
    scroll.add(&app.channels_priv);
    channels_wrapper.add(&scroll);

    let add = Button::new_with_mnemonic("Add _Channel");
    add_class(&add, "add");

    let app_clone = Rc::clone(&app);
    add.connect_clicked(move |_| {
        *app_clone.stack_edit_channel.edit.borrow_mut() = None;
        app_clone.stack_edit_channel.name.set_text("");
        render_mode(&app_clone.stack_edit_channel.mode_bots, 0);
        render_mode(&app_clone.stack_edit_channel.mode_users, common::PERM_READ | common::PERM_WRITE);

        app_clone.stack.set_visible_child(&app_clone.stack_edit_channel.container);
    });

    app.channel_add.add(&add);
    channels_wrapper.add(&app.channel_add);
    app.stack_main.add(&channels_wrapper);
    app.stack_main.add(&Separator::new(Orientation::Horizontal));

    let content = GtkBox::new(Orientation::Vertical, 2);

    let header = GtkBox::new(Orientation::Horizontal, 2);

    add_class(&app.channel_name, "bold");
    app.channel_name.set_property_margin(10);

    header.add(&app.channel_name);

    let toggle_users = Button::new_from_icon_name("user-available", IconSize::Menu.into());
    add_class(&toggle_users, "icon");
    toggle_users.set_hexpand(true);
    toggle_users.set_halign(Align::End);

    let app_clone = Rc::clone(&app);
    toggle_users.connect_clicked(move |_| {
        app_clone.users_revealer.set_reveal_child(!app_clone.users_revealer.get_reveal_child());
    });

    header.add(&toggle_users);

    content.add(&header);
    content.add(&Separator::new(Orientation::Vertical));

    let noread = Label::new("You do not have the read permission in this channel");
    add_class(&noread, "warning");
    app.messages_noread.add(&noread);

    content.add(&app.messages_noread);

    app.messages.set_valign(Align::End);
    app.messages.set_vexpand(true);
    app.messages_scroll.add(&app.messages);

    app.messages_scroll.set_policy(PolicyType::Never, PolicyType::Always);
    app.messages_scroll.set_overlay_scrolling(false);

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
                        println!("requesting more messages");

                        if let Err(err) = synac.session.write(&Packet::MessageList(common::MessageList {
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

                if let Err(err) = synac.session.write(&Packet::MessageUpdate(common::MessageUpdate {
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

    let message_edit_cancel = Button::new_with_mnemonic("_Cancel");
    let app_clone = Rc::clone(&app);
    message_edit_cancel.connect_clicked(move |_| {
        app_clone.message_edit.set_reveal_child(false);
    });
    message_edit.add(&message_edit_cancel);

    app.message_edit.add(&message_edit);
    content.add(&app.message_edit);

    let input = Entry::new();
    input.set_hexpand(true);
    input.set_placeholder_text("Send a message...");

    let typing_duration = Duration::from_secs(common::TYPING_TIMEOUT as u64 / 2); // TODO: const fn
    let typing_last = RefCell::new(Instant::now());

    let app_clone = Rc::clone(&app);
    input.connect_key_press_event(move |_, event| {
        if event.get_keyval() != 65362 {
            // hardcoded value because gdk::enums::key::uparrow doesn't work
            return Inhibit(false);
        }
        if let Some(addr) = *app_clone.connections.current_server.lock().unwrap() {
            app_clone.connections.execute(addr, |result| {
                if result.is_err() { return; }
                let synac = result.unwrap();

                if synac.current_channel.is_none() { return; }
                let channel = synac.current_channel.unwrap();

                if let Some(msg) = synac.messages.get(channel).iter().rev().find(|msg| msg.author == synac.user) {
                    *app_clone.message_edit_id.borrow_mut() = Some(msg.id);
                    app_clone.message_edit_input.set_text(&*String::from_utf8_lossy(&msg.text));
                    app_clone.message_edit.set_reveal_child(true);

                    // Wait until up arrow has been processed and then refocus

                    let app_clone = Rc::clone(&app_clone);
                    gtk::idle_add(move || {
                        app_clone.message_edit_input.grab_focus();
                        Continue(false)
                    });
                }
            });
        }
        Inhibit(false)
    });
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
                        if let Err(err) = synac.session.write(&Packet::Typing(common::Typing {
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
                if text.starts_with('!') {
                    let mut args = parser::parse(&text[1..]);
                    if args.len() < 2 {
                        alert(&app_clone.window, MessageType::Info, "!<user> <command> [args...]");
                        return;
                    }

                    let recipient = args.remove(0);

                    let mut user_id = None;
                    for user in synac.state.users.values() {
                        if user.bot && user.name == recipient {
                            user_id = Some(user.id);
                            break;
                        }
                    }

                    if user_id.is_none() {
                        alert(&app_clone.window, MessageType::Warning, "No bot with that id.");
                        return;
                    }

                    let result = synac.session.write(&Packet::Command(common::Command {
                        args: args,
                        recipient: user_id.unwrap()
                    }));
                    if let Err(err) = result {
                        eprintln!("failed to send packet: {}", err);
                        return;
                    }
                    return;
                }
                if synac.current_channel.is_none() {
                    return;
                }
                let channel = synac.current_channel.unwrap();
                let result = synac.session.write(&Packet::MessageCreate(common::MessageCreate {
                    channel: channel,
                    text: text.into_bytes()
                }));
                if let Err(err) = result {
                    if let Ok(io_err) = err.downcast::<IoError>() {
                        if io_err.kind() != IoErrorKind::BrokenPipe {
                            return;
                        }
                    }

                    let mut stmt = app_clone.db.prepare_cached("SELECT hash, token FROM servers WHERE ip = ?").unwrap();
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
        input.grab_focus();
    });

    app.message_input.add(&input);
    content.add(&app.message_input);

    app.typing.set_xalign(0.0);
    content.add(&app.typing);

    app.stack_main.add(&content);
    app.stack_main.add(&Separator::new(Orientation::Horizontal));

    app.users.set_property_margin(30);
    app.users_revealer.add(&app.users);
    app.stack_main.add(&app.users_revealer);

    app.stack_edit_server.container.set_property_margin(10);

    app.stack_edit_server.name.set_placeholder_text("Server name...");
    app.stack_edit_server.container.add(&app.stack_edit_server.name);
    app.stack_edit_server.container.add(&Label::new("The server name. This can be anything you want it to."));

    app.stack_edit_server.server.set_placeholder_text("Server IP...");
    app.stack_edit_server.container.add(&app.stack_edit_server.server);

    let mut string = String::with_capacity(43 + 4 + 1);
    write!(string, "The server IP address. The default port is {}.", common::DEFAULT_PORT).unwrap();

    app.stack_edit_server.container.add(&Label::new(&*string));

    app.stack_edit_server.hash.set_placeholder_text("Server's certificate hash...");
    app.stack_edit_server.container.add(&app.stack_edit_server.hash);
    app.stack_edit_server.container.add(&Label::new("The server's certificate public key hash.\n\
                               This is to verify nobody is snooping on your connection"));

    let edit_server_controls = GtkBox::new(Orientation::Horizontal, 2);

    let edit_server_cancel = Button::new_with_mnemonic("_Cancel");
    let app_clone = Rc::clone(&app);
    edit_server_cancel.connect_clicked(move |_| {
        app_clone.stack.set_visible_child(&app_clone.stack_main);
    });
    edit_server_controls.add(&edit_server_cancel);

    let edit_server_ok = Button::new_with_mnemonic("_Ok");

    let app_clone = Rc::clone(&app);
    edit_server_ok.connect_clicked(move |_| {
        let name_text   = app_clone.stack_edit_server.name.get_text().unwrap_or_default();
        let server_text = app_clone.stack_edit_server.server.get_text().unwrap_or_default();
        let hash_text   = app_clone.stack_edit_server.hash.get_text().unwrap_or_default();

        let addr = match connections::parse_addr(&server_text) {
            Some(addr) => addr,
            None => return
        };

        app_clone.stack.set_visible_child(&app_clone.stack_main);

        app_clone.db.execute(
            "REPLACE INTO servers (name, ip, hash) VALUES (?, ?, ?)",
            &[&name_text, &addr.to_string(), &hash_text]
        ).unwrap();
        render_servers(&app_clone);
    });

    edit_server_controls.add(&edit_server_ok);
    app.stack_edit_server.container.add(&edit_server_controls);

    app.stack_edit_channel.container.set_property_margin(10);

    app.stack_edit_channel.name.set_placeholder_text("Channel name...");
    app.stack_edit_channel.container.add(&app.stack_edit_channel.name);

    app.stack_edit_channel.container.add(&Label::new("The channel name."));

    let label = Label::new("Default permissions for bots: ");
    label.set_xalign(0.0);
    app.stack_edit_channel.container.add(&label);

    app.stack_edit_channel.container.add(&app.stack_edit_channel.mode_bots);

    let label = Label::new("Default permissions for users: ");
    label.set_xalign(0.0);
    app.stack_edit_channel.container.add(&label);

    app.stack_edit_channel.container.add(&app.stack_edit_channel.mode_users);

    let edit_channel_controls = GtkBox::new(Orientation::Horizontal, 2);

    let edit_channel_cancel = Button::new_with_mnemonic("_Cancel");
    let app_clone = Rc::clone(&app);
    edit_channel_cancel.connect_clicked(move |_| {
        app_clone.stack.set_visible_child(&app_clone.stack_main);
    });
    edit_channel_controls.add(&edit_channel_cancel);

    let edit_channel_ok = Button::new_with_mnemonic("_Ok");

    let app_clone = Rc::clone(&app);
    edit_channel_ok.connect_clicked(move |_| {
        app_clone.stack.set_visible_child(&app_clone.stack_main);
        if let Some(addr) = *app_clone.connections.current_server.lock().unwrap() {
            app_clone.connections.execute(addr, |result| {
                if result.is_err() { return; }
                let synac = result.unwrap();

                let name = app_clone.stack_edit_channel.name.get_text().unwrap_or_default();

                if name.is_empty() {
                    return;
                }

                let packet = if let Some(channel) = *app_clone.stack_edit_channel.edit.borrow() {
                    Packet::ChannelUpdate(common::ChannelUpdate {
                        inner: common::Channel {
                            default_mode_bot: get_mode(&app_clone.stack_edit_channel.mode_bots).unwrap(),
                            default_mode_user: get_mode(&app_clone.stack_edit_channel.mode_users).unwrap(),
                            id: channel,
                            name: name,
                            private: false
                        }
                    })
                } else {
                    Packet::ChannelCreate(common::ChannelCreate {
                        default_mode_bot: get_mode(&app_clone.stack_edit_channel.mode_bots).unwrap(),
                        default_mode_user: get_mode(&app_clone.stack_edit_channel.mode_users).unwrap(),
                        name: name,
                        recipient: None
                    })
                };

                if let Err(err) = synac.session.write(&packet) {
                    eprintln!("error sending packet: {}", err);
                }
            });
        }
    });

    edit_channel_controls.add(&edit_channel_ok);
    app.stack_edit_channel.container.add(&edit_channel_controls);

    app.stack_edit_user.container.set_property_margin(10);

    let app_clone = Rc::clone(&app);
    app.stack_edit_user.radio_none.connect_toggled(move |radio_none| {
        if radio_none.get_active() {
            app_clone.stack_edit_user.mode.set_sensitive(false);
        } else {
            app_clone.stack_edit_user.mode.set_sensitive(true);
        }
    });

    app.stack_edit_user.container.add(&app.stack_edit_user.radio_none);
    app.stack_edit_user.container.add(&app.stack_edit_user.radio_some);

    app.stack_edit_user.container.add(&app.stack_edit_user.mode);

    let edit_user_controls = GtkBox::new(Orientation::Horizontal, 2);

    let edit_user_cancel = Button::new_with_mnemonic("_Cancel");

    let app_clone = Rc::clone(&app);
    edit_user_cancel.connect_clicked(move |_| {
        app_clone.stack.set_visible_child(&app_clone.stack_main);
    });

    edit_user_controls.add(&edit_user_cancel);

    let edit_user_ok = Button::new_with_mnemonic("_Ok");

    let app_clone = Rc::clone(&app);
    edit_user_ok.connect_clicked(move |_| {
        if let Some(addr) = *app_clone.connections.current_server.lock().unwrap() {
            app_clone.connections.execute(addr, |result| {
                if result.is_err() { return; }
                let synac = result.unwrap();

                if synac.current_channel.is_none() { return; }
                let channel = synac.current_channel.unwrap();

                let mode = if app_clone.stack_edit_user.radio_none.get_active() {
                    None
                } else {
                    Some(get_mode(&app_clone.stack_edit_user.mode).unwrap())
                };

                let result = synac.session.write(&Packet::UserUpdate(common::UserUpdate {
                    admin: None,
                    ban: None,
                    channel_mode: Some((channel, mode)),
                    id: app_clone.stack_edit_user.user.borrow().expect("( ͡° ͜ʖ ͡°)")
                }));
                if let Err(result) = result {
                    eprintln!("error sending packet: {}", result);
                }
            });
            app_clone.stack.set_visible_child(&app_clone.stack_main);
        }
    });

    edit_user_controls.add(&edit_user_ok);
    app.stack_edit_user.container.add(&edit_user_controls);

    app.window.add(&app.stack);

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
                let dark = if let Some(settings) = app.window.get_settings() {
                    settings.get_property_gtk_application_prefer_dark_theme()
                } else { false };

                css.load_from_data(if dark {
                    include_bytes!("dark.css")
                } else {
                    include_bytes!("light.css")
                }).map_err(Error::from)
            };
            if let Err(err) = result {
                let string = format!("failed to load css: {}", err);
                alert(&app.window, MessageType::Error, &string);
            }
            StyleContext::add_provider_for_screen(&screen, &css, STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
    }

    app.window.show_all();
    app.window.connect_delete_event(|_, _| {
        gtk::main_quit();
        Inhibit(false)
    });

    gtk::timeout_add(10, move || {
        let mut channels = false;
        let mut messages = false;
        let mut users = false;

        let current_server = *app.connections.current_server.lock().unwrap();

        if let Err(err) = app.connections.try_read(|synac, packet, channel_id| {
            println!("received {:?}", packet);
            if current_server != Some(synac.addr) {
                return;
            }
            let channel = channel_id.and_then(|id| synac.state.channels.get(&id));
            match packet {
                Packet::ChannelDeleteReceive(_) |
                Packet::ChannelReceive(_) => channels = true,
                Packet::MessageDeleteReceive(_) => messages = true,
                Packet::MessageListReceived => {
                    messages = true;
                    scroll_to_bottom(&app);
                }
                Packet::MessageReceive(e) => {
                    messages = e.new;

                    let msg = &e.inner;
                    if e.new && msg.author != synac.user && !app.window.is_active() {
                        if let Some(channel) = channel {
                            if let Some(author) = synac.state.users.get(&msg.author) {
                                let mut stmt = app.db.prepare_cached(
                                    "SELECT COUNT(*) FROM muted WHERE channel = ? AND server = ?"
                                ).unwrap();
                                let count: i64 = stmt.query_row(
                                    &[&(channel.id as i64), &synac.addr.to_string()],
                                    |row| row.get(0)
                                ).unwrap();

                                if count == 0 {
                                    let result =
                                        Notification::new()
                                            .summary(&format!("{} (#{})", author.name, channel.name))
                                            .body(&*String::from_utf8_lossy(&msg.text))
                                            .show();
                                    if let Err(err) = result {
                                        eprintln!("error showing notification: {}", err);
                                    }
                                }
                            }
                        }
                    }
                },
                Packet::UserReceive(_) => users = true,
                _ => {}
            }
        }) {
            eprintln!("receive error: {}", err);
            return Continue(true);
        }

        if let Some(addr) = current_server {
            app.connections.execute(addr, |result| {
                if result.is_err() { return; }
                let synac = result.unwrap();

                if channels {
                    render_channels(&app, Some(synac));
                } else if messages {
                    render_messages(&app, Some(synac));
                } else if users {
                    render_users(&app,    Some(synac));
                }

                if let Some(typing) = synac.typing.check(synac.current_channel, &synac.state) {
                    app.typing.set_text(&typing);
                }
            });
        }

        Continue(true)
    });
    gtk::main();
}
