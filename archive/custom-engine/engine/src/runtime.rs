use crate::game::InputState;
use crate::world::World;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;

/// Type-indexed game state owned by the engine application.
#[derive(Default)]
pub struct Resources {
    values: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

/// Type-indexed event queues. Events remain queued until a system drains them.
#[derive(Default)]
pub struct Events {
    queues: HashMap<TypeId, Box<dyn Any + Send>>,
}

impl Events {
    pub fn send<T>(&mut self, event: T)
    where
        T: Send + 'static,
    {
        self.queues
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(Vec::<T>::new()))
            .downcast_mut::<Vec<T>>()
            .expect("an event type has one queue")
            .push(event);
    }

    pub fn read<T>(&self) -> &[T]
    where
        T: Send + 'static,
    {
        self.queues
            .get(&TypeId::of::<T>())
            .and_then(|queue| queue.downcast_ref::<Vec<T>>())
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn drain<T>(&mut self) -> Vec<T>
    where
        T: Send + 'static,
    {
        self.queues
            .remove(&TypeId::of::<T>())
            .and_then(|queue| queue.downcast::<Vec<T>>().ok())
            .map(|queue| *queue)
            .unwrap_or_default()
    }
}

impl Resources {
    pub fn insert<T>(&mut self, value: T) -> Option<T>
    where
        T: Send + Sync + 'static,
    {
        self.values
            .insert(TypeId::of::<T>(), Box::new(value))
            .and_then(|previous| previous.downcast().ok())
            .map(|previous| *previous)
    }

    pub fn contains<T>(&self) -> bool
    where
        T: Send + Sync + 'static,
    {
        self.values.contains_key(&TypeId::of::<T>())
    }

    pub fn get<T>(&self) -> Option<&T>
    where
        T: Send + Sync + 'static,
    {
        self.values.get(&TypeId::of::<T>())?.downcast_ref()
    }

    pub fn get_mut<T>(&mut self) -> Option<&mut T>
    where
        T: Send + Sync + 'static,
    {
        self.values.get_mut(&TypeId::of::<T>())?.downcast_mut()
    }

    pub fn remove<T>(&mut self) -> Option<T>
    where
        T: Send + Sync + 'static,
    {
        self.values
            .remove(&TypeId::of::<T>())?
            .downcast()
            .ok()
            .map(|value| *value)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Time {
    pub delta_seconds: f32,
    pub fixed_delta_seconds: f32,
    pub elapsed_seconds: f64,
    pub frame_index: u64,
    pub fixed_tick: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Schedule {
    Startup,
    FixedUpdate,
    Update,
}

/// The state available to one game system invocation.
pub struct SystemContext<'a> {
    pub world: &'a mut World,
    pub resources: &'a mut Resources,
    pub events: &'a mut Events,
    pub input: InputState,
    pub time: Time,
    pub schedule: Schedule,
}

pub type SystemResult = Result<(), String>;

pub trait GameSystem: Send {
    fn run(&mut self, context: &mut SystemContext<'_>) -> SystemResult;
}

impl<F> GameSystem for F
where
    F: for<'a> FnMut(&mut SystemContext<'a>) -> SystemResult + Send,
{
    fn run(&mut self, context: &mut SystemContext<'_>) -> SystemResult {
        self(context)
    }
}

pub(crate) struct NamedSystem {
    pub name: String,
    system: Box<dyn GameSystem>,
}

impl NamedSystem {
    pub(crate) fn new(name: impl Into<String>, system: impl GameSystem + 'static) -> Self {
        Self {
            name: name.into(),
            system: Box::new(system),
        }
    }

    pub(crate) fn run(
        &mut self,
        world: &mut World,
        resources: &mut Resources,
        events: &mut Events,
        input: InputState,
        time: Time,
        schedule: Schedule,
    ) -> Result<(), SystemFailure> {
        self.system
            .run(&mut SystemContext {
                world,
                resources,
                events,
                input,
                time,
                schedule,
            })
            .map_err(|message| SystemFailure {
                system: self.name.clone(),
                schedule,
                message,
            })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemFailure {
    pub system: String,
    pub schedule: Schedule,
    pub message: String,
}

impl fmt::Display for SystemFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} system '{}' failed: {}",
            match self.schedule {
                Schedule::Startup => "startup",
                Schedule::FixedUpdate => "fixed-update",
                Schedule::Update => "update",
            },
            self.system,
            self.message
        )
    }
}

impl std::error::Error for SystemFailure {}

#[cfg(test)]
mod tests {
    use super::{Events, Resources};

    #[test]
    fn resources_are_type_indexed_and_replaceable() {
        let mut resources = Resources::default();

        assert_eq!(resources.insert(3_u32), None);
        assert_eq!(resources.insert(String::from("player")), None);
        assert_eq!(resources.get::<u32>(), Some(&3));
        assert_eq!(resources.insert(8_u32), Some(3));
        assert_eq!(resources.remove::<u32>(), Some(8));
        assert!(!resources.contains::<u32>());
        assert_eq!(
            resources.get::<String>().map(String::as_str),
            Some("player")
        );
    }

    #[test]
    fn events_are_typed_and_remain_until_drained() {
        let mut events = Events::default();
        events.send(3_u32);
        events.send(5_u32);
        events.send(String::from("spawned"));

        assert_eq!(events.read::<u32>(), [3, 5]);
        assert_eq!(events.drain::<u32>(), [3, 5]);
        assert!(events.read::<u32>().is_empty());
        assert_eq!(events.drain::<String>(), ["spawned"]);
    }
}
