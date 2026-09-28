use fvid::playback_spool::{Source, Spool, lead_ahead, lead_bytes, source_rate};
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    time::{Duration, Instant},
};

/// A file big enough that the copier cannot have finished before the reader
/// starts asking, and whose bytes are worth comparing exactly.
fn item(name: &str, size: usize) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("fvid-spool-test-{name}"));
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join(format!("{name}.bin"));
    let mut bytes = Vec::with_capacity(size);
    let mut seed = 0x2545_F491u32;
    for index in 0..size {
        if index % 4096 == 0 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        }
        bytes.push(((seed >> ((index % 4) * 8)) as u8).wrapping_add(index as u8));
    }
    fs::write(&path, &bytes).unwrap();
    path
}

fn spooled(path: &PathBuf, lead: u64) -> Spool {
    Spool::open(path, lead).unwrap()
}

/// A megabyte of wrong bytes is not worth printing twice, so say where the two
/// copies part instead of dumping them.
fn differs(got: &[u8], wanted: &[u8]) -> String {
    if got.len() != wanted.len() {
        return format!("length {} against {}", got.len(), wanted.len());
    }
    match got.iter().zip(wanted).position(|(a, b)| a != b) {
        Some(at) => format!("byte {at} is {} where the item has {}", got[at], wanted[at]),
        None => String::new(),
    }
}

/// The temporary a spool of the item named `name` is keeping, while it is open:
/// the window it holds is what a seek has to pay for, and only the file itself
/// says how much that is.
fn temporary_of(name: &str) -> Option<PathBuf> {
    let wanted = format!("{name}bin");
    fs::read_dir(std::env::temp_dir())
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .find(|path| match path.file_name() {
            Some(file) => {
                let file = file.to_string_lossy();
                file.contains("fvid-spool") && file.contains(&wanted)
            }
            None => false,
        })
}

#[test]
fn the_spool_serves_the_same_bytes_as_the_file_it_copies() {
    let path = item("whole", 12 << 20);
    let wanted = fs::read(&path).unwrap();
    let mut source = spooled(&path, 4 << 20);
    let mut got = Vec::new();
    source.read_to_end(&mut got).unwrap();
    assert!(
        differs(&got, &wanted).is_empty(),
        "the spool: {}",
        differs(&got, &wanted)
    );
    // A window never holds more than its lead, so what says the copier did its
    // work is that it ran out of item rather than a byte count.
    assert!(source.exhausted(), "the copier stopped short");
    drop(source);
    let _ = fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_seek_to_the_tail_gives_the_items_own_bytes() {
    let path = item("past", 16 << 20);
    let wanted = fs::read(&path).unwrap();
    let mut source = spooled(&path, 1 << 20);
    // The tail is asked for before the copier can have reached it, which is how
    // a container's end-of-file header arrives on the first open.
    let tail = 15 << 20;
    source.seek(SeekFrom::End(-(tail as i64))).unwrap();
    let mut got = vec![0u8; tail];
    source.read_exact(&mut got).unwrap();
    assert!(
        differs(&got, &wanted[wanted.len() - tail..]).is_empty(),
        "past the spool: {}",
        differs(&got, &wanted[wanted.len() - tail..])
    );
    drop(source);
    let _ = fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_forward_seek_costs_a_window_rather_than_the_item_behind_it() {
    let path = item("follow", 64 << 20);
    let wanted = fs::read(&path).unwrap();
    let mut source = spooled(&path, 2 << 20);
    let jump = 48 << 20usize;
    source.seek(SeekFrom::Start(jump as u64)).unwrap();
    let mut got = vec![0u8; 3 << 20];
    source.read_exact(&mut got).unwrap();
    assert!(
        differs(&got, &wanted[jump..jump + (3 << 20)]).is_empty(),
        "after the jump: {}",
        differs(&got, &wanted[jump..jump + (3 << 20)])
    );
    // The 48 MiB the reader skipped are nobody's business: chasing them would
    // copy, and pay network for, the whole item to reach one byte of it.
    let spool = temporary_of("follow").expect("a spool file for the item");
    let held = fs::metadata(&spool).unwrap();
    let blocks = {
        use std::os::unix::fs::MetadataExt;
        held.blocks() * 512
    };
    assert!(
        blocks < (6 << 20),
        "the spool holds {blocks} bytes of the 64 MiB item after a jump to {jump}"
    );
    drop(source);
    let _ = fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_window_move_does_not_invent_bytes_behind_it() {
    let path = item("move", 32 << 20);
    let wanted = fs::read(&path).unwrap();
    let mut source = spooled(&path, 2 << 20);
    let mut first = vec![0u8; 1 << 20];
    source.read_exact(&mut first).unwrap();
    // The window moves to the jump, giving up what it had copied from the start.
    let jump = 24 << 20usize;
    source.seek(SeekFrom::Start(jump as u64)).unwrap();
    let mut there = vec![0u8; 4 << 20];
    source.read_exact(&mut there).unwrap();
    assert!(
        differs(&there, &wanted[jump..jump + (4 << 20)]).is_empty(),
        "in the new window: {}",
        differs(&there, &wanted[jump..jump + (4 << 20)])
    );
    // And what lies below it is the item again, not the empty file the spool was
    // cut back to.
    source.seek(SeekFrom::Start(0)).unwrap();
    let mut again = vec![0u8; 1 << 20];
    source.read_exact(&mut again).unwrap();
    assert!(
        differs(&again, &wanted[..1 << 20]).is_empty(),
        "below the window: {}",
        differs(&again, &wanted[..1 << 20])
    );
    assert!(
        differs(&first, &again).is_empty(),
        "a rewind after the move: {}",
        differs(&first, &again)
    );
    drop(source);
    let _ = fs::remove_dir_all(path.parent().unwrap());
}

/// The lead a reader is told about has to be measured from the bytes it will
/// actually reach. A camera MP4 keeps its index at the tail, so opening seeks to
/// the end of the file and the window goes there with it; counting the far edge
/// of that window from a reader back at byte zero priced three gigabytes of
/// never-fetched item as picture already on disk, and the first frame was shown
/// on the strength of it.
#[test]
fn a_lead_is_counted_from_where_the_reader_reaches() {
    // Inside the window: what it holds ahead of the reader.
    assert_eq!(lead_ahead(48 << 20, 44 << 20, 52 << 20), 4 << 20);
    // At the far edge: the window is entirely bitten.
    assert_eq!(lead_ahead(52 << 20, 44 << 20, 52 << 20), 0);
    // Behind it, by the whole length of a clip: the copy covers bytes the reader
    // will not ask for until the window comes back to it.
    assert_eq!(lead_ahead(0, 3 << 30, (3 << 30) + (256 << 20)), 0);
    // And ahead of it, which is what the reader does when it jumps forward.
    assert_eq!(lead_ahead(60 << 20, 44 << 20, 52 << 20), 0);
}

/// A window the reader has left behind still holds bytes, just not bytes this
/// reader is standing in front of: the lead has to stay inside the copy.
#[test]
fn a_reader_behind_the_window_is_told_only_what_is_local() {
    let path = item("gap", 64 << 20);
    let mut source = spooled(&path, 4 << 20);
    let handle = source.handle();
    source.seek(SeekFrom::Start(48 << 20)).unwrap();
    let mut there = vec![0u8; 1 << 20];
    source.read_exact(&mut there).unwrap();
    let started = Instant::now();
    while handle.copied() < 4 << 20 && started.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(handle.copied() > 0, "the window never filled at the tail");
    // A step back shorter than a lead gives the copier no reason to move the
    // window, so what is on disk stays what it was.
    source.seek(SeekFrom::Start(45 << 20)).unwrap();
    // The two counters are separate mirrors, so a block the copier lands
    // between the loads legitimately shows in one and not the other. Three
    // megabytes of reader-behind-window are not that.
    assert!(
        handle.ahead() <= handle.copied() + (1 << 20),
        "the reader sat behind a window of {} bytes and was told {} were ahead of it",
        handle.copied(),
        handle.ahead()
    );
    drop(source);
    let _ = fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn rewinding_re_serves_bytes_from_the_local_copy() {
    let path = item("rewind", 8 << 20);
    let wanted = fs::read(&path).unwrap();
    let mut source = spooled(&path, 6 << 20);
    let mut first = vec![0u8; 1 << 20];
    source.read_exact(&mut first).unwrap();
    source.seek(SeekFrom::Start(0)).unwrap();
    let mut again = vec![0u8; 1 << 20];
    source.read_exact(&mut again).unwrap();
    assert!(
        differs(&first, &again).is_empty(),
        "a rewind read: {}",
        differs(&first, &again)
    );
    assert!(
        differs(&again, &wanted[..1 << 20]).is_empty(),
        "rewind: {}",
        differs(&again, &wanted[..1 << 20])
    );
    drop(source);
    let _ = fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn dropping_the_spool_takes_its_temporary_with_it() {
    let path = item("dropped", 2 << 20);
    let spool = spooled(&path, 1 << 20);
    let directory = std::env::temp_dir();
    let mine: Vec<_> = fs::read_dir(&directory)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("droppedbin"))
        .collect();
    assert!(!mine.is_empty(), "no spool file was left to find");
    drop(spool);
    let left: Vec<_> = fs::read_dir(&directory)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("droppedbin"))
        .collect();
    assert!(left.is_empty(), "the spool outlived its reader: {left:?}");
    let _ = fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn an_item_is_still_the_item_when_the_copier_is_held_back() {
    let path = item("held", 3 << 20);
    let wanted = fs::read(&path).unwrap();
    // No lead at all: every byte reaches the reader through the staged block,
    // served out in pieces, which is where a staged offset can go stale.
    let mut source = spooled(&path, 0);
    let mut got = Vec::new();
    loop {
        let mut piece = [0u8; 4096];
        let read = source.read(&mut piece).unwrap();
        if read == 0 {
            break;
        }
        got.extend_from_slice(&piece[..read]);
    }
    assert!(
        differs(&got, &wanted).is_empty(),
        "staged: {}",
        differs(&got, &wanted)
    );
    drop(source);
    let _ = fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn the_buffer_a_lead_costs_is_the_share_of_the_item_it_runs_for() {
    let minute = Duration::from_secs(60);
    // Five seconds of a minute-long item is a twelfth of it.
    assert_eq!(
        lead_bytes(1200, minute, Duration::from_secs(5)),
        100,
        "the lead did not scale with the item"
    );
    // An item shorter than the lead is buffered whole, and one of no stated
    // length is not held back by a wait that can never be answered.
    assert_eq!(
        lead_bytes(900, Duration::from_secs(3), Duration::from_secs(5)),
        900
    );
    assert_eq!(lead_bytes(900, Duration::ZERO, Duration::from_secs(5)), 900);
}

#[test]
fn a_source_answer_for_its_speed_only_when_the_file_is_there() {
    let path = item("rate", 4 << 20);
    let rate = source_rate(&path, 1 << 20).expect("a local file answers for itself");
    assert!(rate > 1.0, "a local read measured {rate} MB/s");
    assert!(source_rate(&PathBuf::from("/no/such/item.bin"), 1 << 20).is_none());
    assert!(source_rate(&path, 0).is_none());
    let _ = fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_window_stays_its_lead_while_the_reader_runs_forward() {
    let path = item("wide", 24 << 20);
    let wanted = fs::read(&path).unwrap();
    let lead = 2u64 << 20;
    let mut source = spooled(&path, lead);
    let mut got = Vec::with_capacity(24 << 20);
    let mut widest = 0u64;
    let mut piece = vec![0u8; 64 << 10];
    loop {
        let read = source.read(&mut piece).unwrap();
        if read == 0 {
            break;
        }
        got.extend_from_slice(&piece[..read]);
        // The spool is a lead of disk, not a download: what the reader has
        // passed has to be given back as the window moves on.
        widest = widest.max(source.copied());
    }
    assert!(
        differs(&got, &wanted).is_empty(),
        "across the window's back edge: {}",
        differs(&got, &wanted)
    );
    assert!(
        widest <= lead + (1 << 20),
        "the window grew to {widest} bytes on a {lead} byte lead"
    );
    drop(source);
    let _ = fs::remove_dir_all(path.parent().unwrap());
}

/// The lead a caller is shown and waited for is the part of the window the
/// reader has not bitten yet, so the copier has to fill it and the reader has to
/// eat it: a number only the copier ever moved would still be showing a block of
/// lead after the item had been read to its end.
#[test]
fn the_lead_is_filled_by_the_copier_and_eaten_by_the_reader() {
    let path = item("slack", 8 << 20);
    let mut source = spooled(&path, 4 << 20);
    let handle = source.handle();
    // A local source is one the reader outruns, so the copier's own fill is
    // looked at before anybody has read: with the reader still at byte zero,
    // whatever is on disk is ahead of it.
    let started = Instant::now();
    while handle.copied() == 0 && started.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        handle.ahead() > 0,
        "the copier put {} bytes on disk and none of them ahead",
        handle.copied()
    );
    let mut got = Vec::new();
    let mut filled = false;
    let mut piece = vec![0u8; 1 << 20];
    loop {
        let read = source.read(&mut piece).unwrap();
        if read == 0 {
            break;
        }
        got.extend_from_slice(&piece[..read]);
        assert!(
            handle.ahead() <= handle.copied(),
            "a lead of {} is longer than the {} byte window it is cut from",
            handle.ahead(),
            handle.copied()
        );
        filled |= handle.ahead() > 0;
    }
    assert!(filled, "the copier never held a byte ahead of the reader");
    assert_eq!(handle.ahead(), 0, "the item is read out; the lead is not");
    assert!(
        handle.copied() > 0,
        "the window gave all its room back before the end"
    );
    assert_eq!(got.len(), 8 << 20);
    drop(source);
    let _ = fs::remove_dir_all(path.parent().unwrap());
}

/// An item shorter than the probe is one request from any source, so a copy has
/// nothing to win - and there is no rate worth believing either: measured over a
/// few hundred bytes it says what the syscalls cost, which is how a small local
/// file came to look slower than a cloud drive and got a spool for it.
#[test]
fn a_short_item_is_read_as_it_is_however_slow_its_probe_looks() {
    // A floor above every rate leaves the size as the only reason to stay away.
    let small = item("short", 12 << 10);
    let (source, handle) = Source::open(&small, f64::INFINITY, 1 << 20).unwrap();
    assert!(
        matches!(source, Source::Plain(_)),
        "a short item was given a spool"
    );
    assert!(handle.is_none(), "a plain source came with a handle");
    drop(source);
    let _ = fs::remove_dir_all(small.parent().unwrap());
    // The same question about an item with a body takes the copy.
    let big = item("long", 4 << 20);
    let (source, handle) = Source::open(&big, f64::INFINITY, 1 << 20).unwrap();
    assert!(
        matches!(source, Source::Spooled(_)),
        "an item with a body was left uncopied"
    );
    assert!(handle.is_some(), "a spooled source has no handle");
    drop(source);
    let _ = fs::remove_dir_all(big.parent().unwrap());
}
