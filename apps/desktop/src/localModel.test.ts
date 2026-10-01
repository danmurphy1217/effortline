import assert from "node:assert/strict";
import test from "node:test";
import { modelErrorMessage } from "./localModel.ts";

void test("local model failures preserve the deterministic result and offer retry", () => {
  assert.match(
    modelErrorMessage("inference_failed"),
    /measured result is still available/,
  );
  assert.match(modelErrorMessage("verification_failed"), /install it again/);
  assert.match(modelErrorMessage("download_failed"), /try again/);
  assert.match(modelErrorMessage("cancelled"), /cancelled/);
});
