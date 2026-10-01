/// <reference types="node" />
import assert from "node:assert/strict";
import test from "node:test";
import {
  investigationProgressMessage,
  type InvestigationProgress,
} from "./investigation.ts";
import type { RunningInvestigationResponse } from "./investigation.ts";

void test("investigation progress names the active stage and elapsed time", () => {
  const opening: InvestigationProgress = {
    version: 1,
    stage: "opening_library",
    elapsed_ms: 12_000,
  };
  assert.match(
    investigationProgressMessage(opening, 12),
    /encrypted activity library/,
  );
  assert.match(investigationProgressMessage(opening, 12), /12 s/);
  assert.match(
    investigationProgressMessage(opening, 12),
    /Keychain permission window/,
  );

  const inference: InvestigationProgress = {
    version: 1,
    stage: "generating_explanation",
    elapsed_ms: 22_000,
  };
  assert.match(
    investigationProgressMessage(inference, 22),
    /local explanation from the evidence — 22 s/,
  );
  assert.doesNotMatch(investigationProgressMessage(inference, 22), /%/);
});

void test("running answer shows pace evidence and does not invent missing heart rate", async () => {
  const { createServer } = await import("vite");
  const { createElement } = await import("react");
  const { renderToStaticMarkup } = await import("react-dom/server");
  const server = await createServer({
    configFile: false,
    server: { middlewareMode: true, ws: false },
    appType: "custom",
  });
  try {
    const { RunningAnswer } = (await server.ssrLoadModule(
      "/src/App.tsx",
    )) as typeof import("./App");
    const activity = (source_id: string, started_at_unix_ms: number) => ({
      source_id,
      started_at_unix_ms,
      duration_seconds: 1500,
      distance_m: 5000,
      pace_seconds_per_km: 300,
      sample_count: 20,
      heart_rate_sample_count: 0,
      median_heart_rate_bpm: null,
    });
    const result: RunningInvestigationResponse = {
      version: 1,
      status: "compared",
      previous_runs: [
        activity("a".repeat(64), Date.UTC(2026, 0, 1)),
        activity("b".repeat(64), Date.UTC(2026, 0, 2)),
        activity("c".repeat(64), Date.UTC(2026, 0, 3)),
      ],
      recent_runs: [
        activity("d".repeat(64), Date.UTC(2026, 0, 4)),
        activity("e".repeat(64), Date.UTC(2026, 0, 5)),
        activity("f".repeat(64), Date.UTC(2026, 0, 6)),
      ],
      pace: {
        previous_median_seconds_per_km: 300,
        recent_median_seconds_per_km: 280,
        change_percent: -6.6667,
      },
      heart_rate: {
        status: "insufficient_coverage",
        previous_qualified_runs: 0,
        recent_qualified_runs: 0,
        required_runs_per_period: 3,
        minimum_samples_per_run: 10,
        minimum_coverage_percent: 50,
      },
      device_history: "missing",
      explanation: null,
      explanation_error: null,
    };
    const html = renderToStaticMarkup(createElement(RunningAnswer, { result }));
    assert.match(html, /5:00\/km/);
    assert.match(html, /4:40\/km/);
    assert.match(html, /6\.7% faster/);
    assert.match(html, /Previous three runs/);
    assert.match(html, /Recent three runs/);
    assert.match(html, /Source citation/);
    assert.match(html, /a{64}/);
    assert.match(html, /Heart-rate comparison is unavailable/);
    assert.match(html, /Not recorded \(0\/20\)/);
    assert.doesNotMatch(html, /because your|caused by/);
  } finally {
    await server.close();
  }
});

void test("running answer states the exact minimum when saved data are thin", async () => {
  const { createServer } = await import("vite");
  const { createElement } = await import("react");
  const { renderToStaticMarkup } = await import("react-dom/server");
  const server = await createServer({
    configFile: false,
    server: { middlewareMode: true, ws: false },
    appType: "custom",
  });
  try {
    const { RunningAnswer } = (await server.ssrLoadModule(
      "/src/App.tsx",
    )) as typeof import("./App");
    const html = renderToStaticMarkup(
      createElement(RunningAnswer, {
        result: {
          version: 1,
          status: "insufficient_data",
          eligible_runs: 2,
          required_runs: 6,
          reason: "too_few_runs",
        },
      }),
    );
    assert.match(html, /2 eligible runs/);
    assert.match(html, /need 6/);
    assert.match(html, /three recent runs/);
    const ambiguous = renderToStaticMarkup(
      createElement(RunningAnswer, {
        result: {
          version: 1,
          status: "insufficient_data",
          eligible_runs: 6,
          required_runs: 6,
          reason: "ambiguous_period_boundary",
        },
      }),
    );
    assert.match(ambiguous, /same start time/);
    assert.match(ambiguous, /won’t report a pace change/);
  } finally {
    await server.close();
  }
});
