/// <reference types="node" />
import assert from "node:assert/strict";
import test from "node:test";
import { saveButtonState, saveProgressMessage } from "./librarySave.ts";

void test("completed saves never retain a busy cursor or an actionable save label", () => {
  for (const [status, label] of [
    ["saved", "Saved"],
    ["already_present", "Already saved"],
  ] as const) {
    const state = saveButtonState(false, { version: 1, status });
    assert.equal(state.cursor, "default");
    assert.equal(state.label, label);
    assert.equal(state.disabled, true);
  }
});

void test("only an active save is busy; errors allow a retry", () => {
  assert.equal(saveButtonState(true, null).label, "Saving…");
  assert.equal(saveButtonState(true, null).cursor, "progress");
  const retry = saveButtonState(false, {
    version: 1,
    status: "error",
    code: "library_secret_unavailable",
  });
  assert.equal(retry.disabled, false);
  assert.equal(retry.label, "Save to library");
  assert.equal(retry.cursor, "default");
});

void test("permission waits name Keychain and do not claim storage work or progress percentages", () => {
  const message = saveProgressMessage(
    { version: 1, status: "started", stage: "keychain_access" },
    65,
  );
  assert.match(message, /Keychain permission window/);
  assert.match(message, /65 s/);
  assert.match(message, /library has not been opened/);
  assert.doesNotMatch(message, /%|Saved/);
});

void test("slow commits stay unconfirmed until the save response arrives", () => {
  const waiting = saveProgressMessage(
    { version: 1, status: "started", stage: "database_commit" },
    30,
  );
  assert.match(waiting, /Committing/);
  assert.match(waiting, /Completion is not yet confirmed/);
  const finished = saveProgressMessage(
    {
      version: 1,
      status: "finished",
      stage: "database_commit",
      elapsed_ms: 30000,
      succeeded: true,
    },
    30,
  );
  assert.match(finished, /Waiting for the next save result/);
  assert.doesNotMatch(finished, /Saved to/);
});

void test("rendered completed states replace the save action and active progress", async () => {
  const { createServer } = await import("vite");
  const { createElement } = await import("react");
  const { renderToStaticMarkup } = await import("react-dom/server");
  const server = await createServer({
    configFile: false,
    server: { middlewareMode: true },
    appType: "custom",
  });
  try {
    const { SaveAction } = (await server.ssrLoadModule(
      "/src/App.tsx",
    )) as typeof import("./App");
    for (const completed of [1024, 8192, 16705]) {
      const html = renderToStaticMarkup(
        createElement(SaveAction, {
          saving: true,
          busy: false,
          saveResponse: null,
          progress: {
            version: 1,
            status: "samples_written",
            completed,
            total: 16705,
            elapsed_ms: 250,
          },
          elapsedSeconds: 2,
          onSave: () => {},
        }),
      );
      assert.match(html, /role="status"/);
      assert.ok(
        html.includes(`${completed.toLocaleString()} of 16,705 written`),
      );
      assert.match(html, /Not yet committed/);
      assert.doesNotMatch(html, /Saved to library|save-complete|%/);
    }
    for (const [status, heading] of [
      ["saved", "Saved to library"],
      ["already_present", "Already in your library"],
    ] as const) {
      const html = renderToStaticMarkup(
        createElement(SaveAction, {
          saving: false,
          busy: false,
          saveResponse: { version: 1, status },
          progress: {
            version: 1,
            status: "finished",
            stage: "database_commit",
            elapsed_ms: 2,
            succeeded: true,
          },
          elapsedSeconds: 20,
          onSave: () => {
            throw new Error("completed saves cannot run again");
          },
        }),
      );
      assert.ok(html.includes(heading));
      assert.match(html, /role="status"/);
      assert.doesNotMatch(
        html,
        /<button|Saving|Waiting|cursor:progress|cursor:wait/,
      );
    }
  } finally {
    await server.close();
  }
});
