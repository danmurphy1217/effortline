/// <reference types="node" />
import assert from "node:assert/strict";
import test from "node:test";
import {
  batchCommandErrorMessage,
  batchFileErrorMessage,
  saveButtonState,
  saveProgressMessage,
} from "./librarySave.ts";

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
  assert.match(finished, /Waiting for the result/);
  assert.doesNotMatch(finished, /Saved to/);
});

void test("batch FIT and storage failures have useful safe messages", () => {
  assert.match(batchFileErrorMessage("fit_corrupt"), /checksum/);
  assert.match(batchFileErrorMessage("fit_too_large"), /16 MiB/);
  assert.match(batchFileErrorMessage("library_secret_unavailable"), /Keychain/);
  assert.match(
    batchFileErrorMessage("library_secret_unavailable"),
    /Always Allow/,
  );
  assert.match(
    batchFileErrorMessage("library_secret_unavailable"),
    /Choose files to retry/,
  );
  assert.match(batchFileErrorMessage("file_read_failed"), /file access/);
  assert.match(
    batchCommandErrorMessage("randomness_unavailable"),
    /secure import session/,
  );
});

void test("rendered completed states replace the save action and active progress", async () => {
  const { createServer } = await import("vite");
  const { createElement } = await import("react");
  const { renderToStaticMarkup } = await import("react-dom/server");
  const server = await createServer({
    configFile: false,
    server: { middlewareMode: true, ws: false },
    appType: "custom",
  });
  try {
    const { BatchFileList, BatchProgressView, BatchRetryAction, SaveAction } =
      (await server.ssrLoadModule("/src/App.tsx")) as typeof import("./App");
    for (const status of [
      "already_present",
      "checking",
      "unavailable",
    ] as const) {
      const html = renderToStaticMarkup(
        createElement(SaveAction, {
          saving: false,
          busy: false,
          saveResponse: null,
          libraryCheck:
            status === "already_present" ? { version: 1, status } : { status },
          progress: null,
          elapsedSeconds: 0,
          onSave: () => {
            throw new Error("save must not be offered");
          },
        }),
      );
      assert.doesNotMatch(
        html,
        /Save to library|This preview has not been saved/,
      );
      if (status === "already_present") {
        assert.match(html, /Already in your library/);
        assert.doesNotMatch(html, /<button/);
      } else if (status === "checking") {
        assert.match(html, /Checking your library/);
        assert.doesNotMatch(html, /<button/);
      } else {
        assert.match(html, /Check library again/);
      }
    }
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
    const batchHtml = renderToStaticMarkup(
      createElement(BatchFileList, {
        files: [
          {
            index: 1,
            name: "saved.fit",
            status: "saved",
            code: null,
            activity: {
              sport: "running",
              duration_seconds: 1234,
              distance_m: 10000,
              sample_count: 500,
            },
          },
          {
            index: 2,
            name: "duplicate.fit",
            status: "already_present",
            code: null,
            activity: {
              sport: "running",
              duration_seconds: 1234,
              distance_m: 10000,
              sample_count: 500,
            },
          },
          {
            index: 3,
            name: "broken.fit",
            status: "failed",
            code: "fit_corrupt",
            activity: null,
          },
          {
            index: 4,
            name: "later.fit",
            status: "not_imported",
            code: null,
            activity: null,
          },
        ],
      }),
    );
    assert.match(batchHtml, /saved\.fit/);
    assert.match(batchHtml, /Saved to your encrypted library/);
    assert.match(batchHtml, /Already in your library/);
    assert.match(batchHtml, /checksum/);
    assert.match(batchHtml, /cancelled the batch/);
    assert.match(batchHtml, /Running/);
    assert.match(batchHtml, /20:34/);
    assert.match(batchHtml, /10.00 km/);
    assert.match(batchHtml, /500/);
    const retryHtml = renderToStaticMarkup(
      createElement(BatchRetryAction, {
        files: [
          {
            index: 1,
            name: "keychain-blocked.fit",
            status: "failed",
            code: "library_secret_unavailable",
            activity: null,
          },
          {
            index: 2,
            name: "not-started.fit",
            status: "not_imported",
            code: "library_secret_unavailable",
            activity: null,
          },
        ],
        disabled: false,
        onRetry: () => {},
      }),
    );
    assert.match(retryHtml, /Choose files to retry/);
    assert.match(retryHtml, /will not be added again/);
    const noRetryHtml = renderToStaticMarkup(
      createElement(BatchRetryAction, {
        files: [
          {
            index: 1,
            name: "saved.fit",
            status: "saved",
            code: null,
            activity: null,
          },
          {
            index: 2,
            name: "duplicate.fit",
            status: "already_present",
            code: null,
            activity: null,
          },
        ],
        disabled: false,
        onRetry: () => {},
      }),
    );
    assert.equal(noRetryHtml, "");
    const fileProgressHtml = renderToStaticMarkup(
      createElement(BatchProgressView, {
        processedFiles: 1,
        totalFiles: 3,
        active: true,
        saveProgress: {
          version: 1,
          status: "samples_written",
          completed: 1024,
          total: 16705,
          elapsed_ms: 500,
        },
      }),
    );
    assert.match(fileProgressHtml, /1 of 3 files processed/);
    assert.match(fileProgressHtml, /value="1" max="3"/);
    assert.match(fileProgressHtml, /1,024 of 16,705/);
    assert.match(fileProgressHtml, /value="1024" max="16705"/);
    assert.match(fileProgressHtml, /Not yet committed/);
    const indeterminateHtml = renderToStaticMarkup(
      createElement(BatchProgressView, {
        processedFiles: 0,
        totalFiles: 2,
        active: true,
        saveProgress: {
          version: 1,
          status: "started",
          stage: "fit_parsing",
        },
      }),
    );
    const activityBar = indeterminateHtml.match(
      /<div class="batch-stage-progress"[^>]*>/,
    )?.[0];
    assert.ok(activityBar);
    assert.match(activityBar, /aria-label="Current file is being processed"/);
    assert.doesNotMatch(activityBar, /aria-valuenow|value=/);
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
