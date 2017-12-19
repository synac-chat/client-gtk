use std::collections::HashMap;
use std::time::{Duration, Instant};
use synac::{common, State};

pub struct Typing {
    last_checked: Instant,
    people: HashMap<(usize, usize), Instant>
}
impl Typing {
    pub fn new() -> Self {
        Typing {
            last_checked: Instant::now(),
            people: HashMap::new()
        }
    }
    pub fn insert(&mut self, author: usize, channel: usize) {
        self.people.insert((author, channel), Instant::now());
    }
    pub fn check(&mut self, channel: Option<usize>, state: &State) -> Option<String> {
        let typing_check = Duration::from_secs(1); // TODO: const fn
        if self.last_checked.elapsed() < typing_check {
            return None;
        }
        self.last_checked = Instant::now();
        let typing_timeout = Duration::from_secs(common::TYPING_TIMEOUT as u64); // TODO: const fn

        self.people.retain(|_, time| time.elapsed() < typing_timeout);

        let people: Vec<_> = self.people.keys()
            .filter_map(|&(author, channel2)| {
                if Some(channel2) != channel {
                    return None;
                }

                state.users.get(&author).map(|user| &user.name)
            })
            .collect();

        Some(match people.len() {
            n if n > 500 => String::from("(╯°□°）╯︵ ┻━┻"),
            n if n > 100 => String::from("A crap ton of people are typing"),
            n if n > 50 => String::from("Over 50 people are typing"),
            n if n > 10 => String::from("Over 10 people are typing"),
            n if n > 3 => String::from("Several people are typing"),
            3 => format!("{}, {} and {} are typing", people[0], people[1], people[2]),
            2 => format!("{} and {} are typing", people[0], people[1]),
            1 => format!("{} is typing", people[0]),
            _ => String::new()
        })
    }
}
