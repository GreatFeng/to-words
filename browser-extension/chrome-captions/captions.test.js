const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");
const script = fs.readFileSync(__dirname + "/captions.js", "utf8");

function detect(videos) {
  const context = {
    document: { querySelectorAll: () => videos, documentElement: { lang: "zh-CN" } },
    chrome: { runtime: { sendMessage: () => Promise.resolve() } },
    setInterval: () => 0,
  };
  vm.createContext(context);
  vm.runInContext(script, context);
  return vm.runInContext("currentCaption()", context);
}

const track = { kind: "subtitles", mode: "disabled", language: "zh-CN", activeCues: [{ text: "你好" }] };
const video = { paused: false, ended: false, muted: false, volume: 1, readyState: 4,
  clientWidth: 640, clientHeight: 360, textTracks: [track] };
assert.equal(detect([video]).text, "你好");
assert.equal(track.mode, "hidden");
assert.equal(detect([{ ...video, textTracks: [] }]).has_track, false);
assert.equal(detect([{ ...video, paused: true }]).playing, false);
console.log("Chrome text track selection passed");
