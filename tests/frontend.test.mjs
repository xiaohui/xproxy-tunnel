import test from "node:test";
import assert from "node:assert/strict";
import {
  defaultConfig,
  validateConfig,
  statusView,
  formatMetric,
  acceptsPoll,
  canAdvance,
  useFixture,
} from "../src/model.mjs";
test("desired state never substitutes for actual connection", () => {
  const v = statusView({ phase: "reconnecting", desired_on: true });
  assert.equal(v.on, true);
  assert.equal(v.connected, false);
  assert.equal(v.label, "正在重连");
});
test("unknown metrics remain unknown", () => {
  assert.equal(formatMetric(null, "latency"), "—");
  assert.equal(formatMetric(1200, "connect"), "1.2 秒");
  assert.equal(formatMetric(86, "latency"), "86 毫秒");
});
test("configuration validates real SSH inputs and distinct ports", () => {
  assert.match(validateConfig(defaultConfig), /服务器/);
  assert.equal(
    validateConfig({
      ...defaultConfig,
      host: "example.org",
      username: "ubuntu",
      key_path: "/tmp/key",
    }),
    "",
  );
  assert.match(
    validateConfig({
      ...defaultConfig,
      host: "x",
      username: "u",
      key_path: "/k",
      http_port: 1080,
    }),
    /不同/,
  );
});
test("stale polling generations and unmounted responses are rejected", () => {
  assert.equal(acceptsPoll(1, 2, true), false);
  assert.equal(acceptsPoll(2, 2, false), false);
  assert.equal(acceptsPoll(2, 2, true), true);
});

test("settings scope requires valid saved SSH configuration", () => {
  const valid = {
    ...defaultConfig,
    host: "example.org",
    username: "ubuntu",
    key_path: "/tmp/key",
  };
  assert.equal(canAdvance(valid, false), false);
  assert.equal(canAdvance(defaultConfig, true), false);
  assert.equal(canAdvance(valid, true), true);
});
test("connected fixtures can never override native runtime", () => {
  assert.equal(useFixture(true, "connected"), false);
  assert.equal(useFixture(false, "connected"), true);
  assert.equal(useFixture(false, null), false);
});
test("a mutation epoch rejects earlier status responses and accepts later polls", () => {
  let epoch = 0;
  const pendingPollEpoch = epoch;
  epoch++;
  assert.equal(acceptsPoll(pendingPollEpoch, epoch, true), false);
  assert.equal(acceptsPoll(epoch, epoch, true), true);
});
test("frontend matches native host username port and probe constraints", () => {
  const valid = {
    ...defaultConfig,
    host: "2001:db8::1",
    username: "ubuntu.user_1",
    key_path: "/tmp/key",
  };
  assert.equal(validateConfig(valid), "");
  assert.match(
    validateConfig({ ...valid, probe_url: "http://example.org" }),
    /HTTPS/,
  );
  assert.match(
    validateConfig({ ...valid, probe_url: "https://user:pass@example.org" }),
    /用户名和密码/,
  );
  assert.match(validateConfig({ ...valid, socks_port: 1023 }), /1024/);
  assert.match(validateConfig({ ...valid, username: "-ubuntu" }), /用户名/);
  assert.match(validateConfig({ ...valid, username: "用户" }), /用户名/);
  assert.match(
    validateConfig({ ...valid, host: "bad_name.example" }),
    /服务器/,
  );
  assert.match(
    validateConfig({ ...valid, key_path: "id_ed25519" }),
    /绝对路径/,
  );
  assert.equal(
    validateConfig({ ...valid, key_path: "C:\\Users\\demo\\key" }),
    "",
  );
});
