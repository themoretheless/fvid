"""Prepare a reviewable clock-hold patch without mutating the running baseline."""
from pathlib import Path
import hashlib,json,re,difflib,tempfile,subprocess
q=Path(__file__).resolve().parent;p=Path('src/player.rs');original=p.read_bytes();expected=json.loads((q/'before-source.json').read_text())['src/player.rs'];assert hashlib.sha256(original).hexdigest()==expected
s=original.decode()
s=s.replace('    paused_picture_time: Option<Duration>,','    paused_picture_time: Option<Duration>,\n    /// Media phase held until missing picture input becomes presentable.\n    buffered_picture_time: Option<Duration>,',1)
s=s.replace('            paused_picture_time: None,','            paused_picture_time: None,\n            buffered_picture_time: None,',1)
s,n=re.subn(r'(?m)^([ \t]*)self\.terminal_clock = None;$',r'\1self.terminal_clock = None;\n\1self.buffered_picture_time = None;',s);assert n==6,n
s=s.replace('            self.terminal_clock = Some(stop);','            self.terminal_clock = Some(stop);\n            self.buffered_picture_time = None;',1)
s=s.replace('        if self.ended { return Self::interval_time(end, scale); }','        if self.ended { return Self::interval_time(end, scale); }\n        if let Some(at) = self.buffered_picture_time { return Some(at); }',1)
needle='        Self::interval_time(nanos, 1_000_000_000)\n    }\n\n    /// Spend an explicit bound'
assert needle in s
s=s.replace(needle,'''        let at = Self::interval_time(nanos, 1_000_000_000)?;
        // A late UI callback still follows wall time when decoded pictures are
        // ready. Missing input cannot spend media beyond the visible interval.
        Some(if !self.picture_audio_clock && (self.awaiting_picture() || self.buffering) { at.min(end) } else { at })
    }

    fn awaiting_picture(&self) -> bool {
        self.queued.is_none() && self.playback.as_ref().is_some_and(|p| p.filled() == 0)
    }

    fn deadline_for_phase(end: Duration, phase: Duration, rate: u32, now: Instant) -> Instant {
        if end >= phase { now.checked_add(paced_period(end - phase, rate)).unwrap_or(now) }
        else { now.checked_sub(paced_period(phase - end, rate)).unwrap_or(now) }
    }

    fn hold_picture_clock(&mut self) {
        let active_audio = self.audio.is_some() && !self.audio_ended;
        if self.playback.is_none() || self.presented.is_none() || self.ended
            || self.seek_preview || self.step > 0 || active_audio
        {
            self.buffered_picture_time = None;
            return;
        }
        if self.paused || self.buffered_picture_time.is_some() { return; }
        if self.buffering
            && let Some(end) = self.interval.and_then(|(_, end, scale)| Self::interval_time(end, scale))
        {
            // A completed/failed device can already be beyond the last video
            // interval. Retain its published phase instead of rewinding it.
            self.buffered_picture_time = Some(if self.picture_audio_clock {
                self.presentation_clock().unwrap_or(end).max(end)
            } else { end });
        }
    }

    /// Spend an explicit bound''',1)
s=s.replace('        self.poll_audio();\n        if self.playback.is_none() {','        self.poll_audio();\n        self.hold_picture_clock();\n        if self.playback.is_none() {',1)
needle='        // Replace expired pictures with newer decoded ones before uploading.';assert needle in s
s=s.replace(needle,'''        if self.queued.is_some()
            && let Some(phase) = self.buffered_picture_time
            && let Some(end) = self.interval.and_then(|(_, end, scale)| Self::interval_time(end, scale))
        {
            // Resume the held phase as soon as real data arrives; a new startup
            // lead would move the visible clock backwards before presentation.
            self.deadline = Self::deadline_for_phase(end, phase, self.rate_milli, now);
        }
        // Replace expired pictures with newer decoded ones before uploading.''',1)
s=s.replace('                let sought_phase = if self.seek_preview && !stepping {','                let buffered_phase = self.buffered_picture_time;\n                let sought_phase = if self.seek_preview && !stepping {',1)
s=s.replace('                    && let Some(phase) = sought_phase\n','                    && let Some(phase) = sought_phase.or(buffered_phase)\n',1)
needle='                    now.checked_add(paced_period(end.saturating_sub(phase), self.rate_milli))\n                        .unwrap_or(now)';assert needle in s
s=s.replace(needle,'                    Self::deadline_for_phase(end, phase, self.rate_milli, now)',1)
needle='''                };
                if stepping {
                    self.step -= 1;''';assert needle in s
s=s.replace(needle,'''                };
                if let Some(phase) = buffered_phase
                    && self.interval.and_then(|(_, end, scale)| Self::interval_time(end, scale))
                        .is_none_or(|end| end > phase)
                {
                    self.buffered_picture_time = None;
                }
                if stepping {
                    self.step -= 1;''',1)
patch=''.join(difflib.unified_diff(original.decode().splitlines(True),s.splitlines(True),fromfile='a/src/player.rs',tofile='b/src/player.rs',n=0));(q/'candidate.patch').write_text(patch)
with tempfile.TemporaryDirectory(prefix='fvid-io-candidate-proof-')as tmp:
 d=Path(tmp);(d/'src').mkdir();(d/'src/player.rs').write_bytes(original);subprocess.run(['git','apply','--unidiff-zero',str((q/'candidate.patch').resolve())],cwd=d,check=True);assert(d/'src/player.rs').read_text()==s
(q/'candidate-plan.json').write_text(json.dumps({'base_source_sha256':expected,'candidate_source_sha256':hashlib.sha256(s.encode()).hexdigest(),'candidate_patch':'candidate.patch','byte_identical_reconstruction':True,'production_mutated_by_preparation':False,'scope':'Freeze genuine empty-queue input waits while retaining wall progression over ready prefetched pictures;retain and reanchor media phase across recovery;clear on navigation/source changes;preserve audio ownership and earlier terminal clock.'},indent=2)+'\n');print('candidate patch prepared;live baseline source untouched')
