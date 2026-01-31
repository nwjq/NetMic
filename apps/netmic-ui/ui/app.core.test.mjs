import test from "node:test";
import assert from "node:assert/strict";
import { createMockAdapter, defaultSnapshot } from "./app.core.js";

test("default snapshot uses MVP defaults", () => {
  const snapshot = defaultSnapshot();
  assert.equal(snapshot.mode, "client");
  assert.equal(snapshot.status, "idle");
  assert.equal(snapshot.client_config.sample_rate_hz, 48000);
  assert.equal(snapshot.client_config.chunk_ms, 20);
  assert.equal(snapshot.client_config.opus_bitrate_kbps, 48);
  assert.equal(snapshot.effective.codec, "opus");
  assert.ok(Array.isArray(snapshot.logs));
  assert.ok(snapshot.logs.length > 0);
});

test("mock adapter start/stop reflects mode", async () => {
  const adapter = createMockAdapter();
  let snapshot = await adapter.setMode("server");
  assert.equal(snapshot.mode, "server");
  snapshot = await adapter.start();
  assert.equal(snapshot.status, "listening");
  assert.equal(snapshot.status_note, "等待客户端连接");
  snapshot = await adapter.stop();
  assert.equal(snapshot.status, "idle");

  snapshot = await adapter.setMode("client");
  snapshot = await adapter.start();
  assert.equal(snapshot.status, "streaming");
  assert.equal(snapshot.status_note, "模拟推流中");
  await adapter.stop();
});

test("mock adapter setClientConfig updates effective params", async () => {
  const adapter = createMockAdapter();
  let snapshot = await adapter.setClientConfig({
    codec: "pcm16",
    opus_bitrate_kbps: 96,
    sample_rate_hz: 44100,
    chunk_ms: 40,
  });
  assert.equal(snapshot.effective.codec, "pcm16");
  assert.equal(snapshot.effective.opus_bitrate_kbps, null);
  assert.equal(snapshot.effective.sample_rate_hz, 44100);
  assert.equal(snapshot.effective.chunk_ms, 40);
  await adapter.stop();
});

test("mock adapter emits snapshot on config change", async () => {
  const adapter = createMockAdapter();
  let received = null;
  adapter.onSnapshot((snapshot) => {
    received = snapshot;
  });
  await adapter.setClientConfig({ server_addr: "10.0.0.8" });
  assert.ok(received);
  assert.equal(received.client_config.server_addr, "10.0.0.8");
  await adapter.stop();
});
