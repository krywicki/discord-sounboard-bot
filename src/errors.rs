use serenity::all::{ChannelId, GuildId};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AudioError {
    #[error("Audio Track not found - {track}")]
    AudioTrackNotFound { track: String },
    #[error("Bot not in voice channel.")]
    NotInVoiceChannel,
}