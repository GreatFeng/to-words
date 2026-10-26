// Observe native HTMLMediaElement.textTracks. Sites that draw their own
// subtitles are handled by to_words' local audio recognition fallback.
function currentCaption() {
  const videos = [...document.querySelectorAll("video")].filter(video =>
    !video.paused && !video.ended && !video.muted && video.volume > 0 && video.readyState >= 2);
  const video = videos.sort((a, b) => b.clientWidth * b.clientHeight - a.clientWidth * a.clientHeight)[0];
  if (!video) return { playing: false, has_track: false, text: "" };
  const tracks = [...video.textTracks].filter(track => track.kind === "subtitles" || track.kind === "captions");
  const active = tracks.find(track => track.mode === "showing") || tracks.find(track => track.mode === "hidden");
  // If the site exposes a track but has not enabled it, read it invisibly.
  const track = active || tracks.find(track => track.language === document.documentElement.lang) || tracks[0];
  if (track && track.mode === "disabled") track.mode = "hidden";
  const text = track?.activeCues ? [...track.activeCues].map(cue => cue.text || "").join("\n") : "";
  return { playing: true, has_track: Boolean(track), text: text.slice(0, 2000) };
}

let previous = "";
setInterval(() => {
  const caption = currentCaption();
  if (!caption.playing) return;
  const encoded = JSON.stringify(caption);
  if (encoded === previous && !caption.has_track) return;
  previous = encoded;
  chrome.runtime.sendMessage({ type: "caption", caption }).catch(() => {});
}, 600);
