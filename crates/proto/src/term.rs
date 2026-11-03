//! A terminal's data stream, `Stream::Pty`, as typed pieces: each is one JSON line naming it, then the bytes it carries.

use std::io;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::line;

pub(crate) type Bytes = Arc<[u8]>;

/// The lines a terminal keeps above its screen, on the server and in every client.
pub const HISTORY_LINES: usize = 10_000;

/// The terminal's history and screen at cols×rows, its cursor and modes included, drawn by writing `bytes` to a
/// terminal reset to cols×rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    pub cols: u16,
    pub rows: u16,
    pub bytes: Bytes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    Screen(Screen),
    /// Output the program wrote since the piece before.
    Output(Bytes),
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "piece", rename_all = "snake_case", deny_unknown_fields)]
enum Header {
    Screen { cols: u16, rows: u16, bytes: usize },
    Output { bytes: usize },
}

impl Piece {
    pub async fn write_to<W: AsyncWrite + Unpin>(&self, write: &mut W) -> io::Result<()> {
        let (header, bytes) = match self {
            Self::Screen(Screen { cols, rows, bytes }) => {
                (Header::Screen { cols: *cols, rows: *rows, bytes: bytes.len() }, bytes)
            }
            Self::Output(bytes) => (Header::Output { bytes: bytes.len() }, bytes),
        };
        write.write_all(line(&header).as_bytes()).await?;
        write.write_all(bytes).await
    }

    /// `None` once the stream has ended.
    pub async fn read_from<R: AsyncBufRead + Unpin>(read: &mut R) -> io::Result<Option<Self>> {
        let mut header = Vec::new();
        if read.read_until(b'\n', &mut header).await? == 0 {
            return Ok(None);
        }
        let header: Header =
            serde_json::from_slice(&header).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let (Header::Screen { bytes: length, .. } | Header::Output { bytes: length }) = header;
        let mut bytes = vec![0; length];
        read.read_exact(&mut bytes).await?;
        Ok(Some(match header {
            Header::Screen { cols, rows, .. } => Self::Screen(Screen { cols, rows, bytes: bytes.into() }),
            Header::Output { .. } => Self::Output(bytes.into()),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pieces_round_trip_and_the_stream_ends_after_the_last() {
        let pieces = [
            Piece::Screen(Screen { cols: 47, rows: 12, bytes: Bytes::from(&b"\x1b[1mbold\n"[..]) }),
            Piece::Output(Bytes::from(&b"\xe2\x94"[..])),
        ];
        let mut wire = Vec::new();
        for piece in &pieces {
            piece.write_to(&mut wire).await.unwrap();
        }
        let mut read = &wire[..];
        for piece in pieces {
            assert_eq!(Piece::read_from(&mut read).await.unwrap(), Some(piece));
        }
        assert_eq!(Piece::read_from(&mut read).await.unwrap(), None);
    }
}
