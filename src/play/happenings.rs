//! What's happening in play, for trying it out: each [`Happened`] is written to the
//! log. Not shown in the game.

use {super::Stage,
     bevy::prelude::*};

/// Something worth telling whoever is trying the game out.
#[derive(Message)]
pub struct Happened(pub String);

/// Writes each thing that happened to the log.
fn log(mut happened: MessageReader<Happened>) {
  happened.read().for_each(|Happened(text)| info!("{text}"))
}

/// Tells of each stage starting. The rescue tells how it ended itself.
fn tell_stage(stage: &'static str) -> impl Fn(MessageWriter<Happened>) {
  move |mut happened: MessageWriter<Happened>| {
    happened.write(Happened(format!("{stage} started")));
  }
}

pub fn plugin(app: &mut App) {
  app
    .add_message::<Happened>()
    .add_systems(OnEnter(Stage::Snowfall), tell_stage("Snowfall"))
    .add_systems(OnEnter(Stage::Briefing), tell_stage("Briefing"))
    .add_systems(OnEnter(Stage::Rescue), tell_stage("Rescue"));
  app.add_systems(Update, log.run_if(in_state(crate::Mode::Play)));
}
