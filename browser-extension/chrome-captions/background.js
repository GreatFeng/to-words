// Content scripts only report videos that are actually playing and not muted.
// A background tab can keep playing sound, so do not require tab.active.
chrome.runtime.onMessage.addListener((message, sender) => {
  if (message?.type !== "caption" || !sender.tab) return;
  const caption = message.caption;
  if (!caption || typeof caption.text !== "string" || typeof caption.has_track !== "boolean"
      || typeof caption.playing !== "boolean") return;
  fetch("http://127.0.0.1:47836/caption", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(caption),
  }).catch(() => {});
});
