// A browser-level pin for the Run tests shortcut in web/interview.js:
// Cmd+Enter or Ctrl+Enter anywhere on the page but another text field, while
// the interview is on.
import { after, before, test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import {
  DEFAULT_RUNTIME_CONFIG,
  launchChromium,
  startStaticServer,
} from "./source.js";

let browser = null;
let server = null;
let base = "";

before(async () => {
  browser = await launchChromium();
  if (!browser) return;
  ({ server, base } = await startStaticServer({
    runtimeConfig: DEFAULT_RUNTIME_CONFIG,
  }));
});

after(async () => {
  await browser?.close();
  await new Promise((closed) => (server ? server.close(closed) : closed()));
});

/// The interview page in JavaScript, with focus in the editor and every run of
/// the tests counted. JavaScript runs in the page, so a run finishes without
/// booting Pyodide.
///
/// Runs are counted, not clicks. A handler that called runTests() directly
/// would start a run without any click, so a click count would miss exactly
/// the second run the disabled button is there to stop. "Running..." is
/// written by runTests alone, once per run.
///
/// It also records whether the last Enter press had its default taken. A
/// window listener runs after the page's own on document, so it sees the
/// answer. The newline check alone cannot stand in for this: macOS Chromium
/// types no newline for either chord in a textarea, so a dropped
/// preventDefault passed every assertion there.
///
/// `started` hides the media preflight, which is how the shortcut tells
/// whether the interview has begun. `media` opens the page in a browser with a
/// fake microphone and camera instead, for a case that clears the preflight
/// for real.
async function interviewPage(t, { started = true, media = false } = {}) {
  if (!browser) {
    t.skip("playwright chromium unavailable");
    return null;
  }
  const page = await (media ? await mediaBrowser(t) : browser).newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(`${base}/interview.html?problem=chargeback-pair-match`, {
    waitUntil: "domcontentloaded",
  });
  // The placeholder is set from inside bindEvents, after the shortcut is
  // registered. Waiting on the editor would not do: it carries no disabled
  // attribute, so it looks ready before any listener exists, and a test that
  // expects no run would pass against a page that never listened.
  await page.waitForFunction(
    () => document.querySelector("#candidate-case-input").placeholder !== "",
  );
  await page.evaluate((started) => {
    document.querySelector('[data-language="javascript"]').click();
    if (started) document.querySelector("#audio-check").hidden = true;
    globalThis.runs = 0;
    const run = document.querySelector("#run-tests");
    new MutationObserver((records) => {
      for (const record of records)
        for (const node of record.addedNodes)
          if (node.textContent === "Running...") globalThis.runs++;
    }).observe(run, { childList: true });
    window.addEventListener("keydown", (event) => {
      if (event.key === "Enter")
        globalThis.enterDefaultPrevented = event.defaultPrevented;
    });
    document.querySelector("#editor").focus();
  }, started);
  return { page, errors };
}

const runs = (page) => page.evaluate(() => globalThis.runs);
const enterDefaultPrevented = (page) =>
  page.evaluate(() => globalThis.enterDefaultPrevented);

/// Waits until Run tests is enabled. The disabled button refuses a press while
/// a run is going or while the interview is paused, so a test that expects a
/// run waits here before pressing, or a press that came too soon would fail
/// it. This wait skips past that refusal, so the in-flight and paused cases
/// press without it, and they are what pin the refusal.
const runButtonReady = (page) =>
  page.waitForFunction(() => !document.querySelector("#run-tests").disabled);

/// A browser of this test's own with a fake microphone and camera, closed when
/// the test ends. Only the paused case needs one: pausing needs a running
/// clock, and only the real preflight starts it. With the preflight merely
/// hidden the deadline is still zero, so the timer tick a pause makes ends the
/// interview, and a press taken then is refused as ended, not as paused.
async function mediaBrowser(t) {
  const { chromium } = await import("playwright");
  const directory = await mkdtemp(join(tmpdir(), "codetrial-shortcut-audio-"));
  const audioFile = join(directory, "microphone.wav");
  const sampleRate = 16000;
  const samples = sampleRate * 3;
  const wav = Buffer.alloc(44 + samples * 2);
  wav.write("RIFF", 0);
  wav.writeUInt32LE(wav.length - 8, 4);
  wav.write("WAVEfmt ", 8);
  wav.writeUInt32LE(16, 16);
  wav.writeUInt16LE(1, 20);
  wav.writeUInt16LE(1, 22);
  wav.writeUInt32LE(sampleRate, 24);
  wav.writeUInt32LE(sampleRate * 2, 28);
  wav.writeUInt16LE(2, 32);
  wav.writeUInt16LE(16, 34);
  wav.write("data", 36);
  wav.writeUInt32LE(samples * 2, 40);
  for (let sample = 0; sample < samples; sample++)
    wav.writeInt16LE(
      Math.round(12000 * Math.sin((2 * Math.PI * 440 * sample) / sampleRate)),
      44 + sample * 2,
    );
  await writeFile(audioFile, wav);
  const media = await chromium.launch({
    args: [
      "--use-fake-device-for-media-stream",
      "--use-fake-ui-for-media-stream",
      `--use-file-for-fake-audio-capture=${audioFile}`,
    ],
  });
  t.after(async () => {
    await media.close();
    await rm(directory, { recursive: true, force: true });
  });
  return media;
}

/// The preflight cleared the way a candidate clears it, as clearMediaGate in
/// scripts/browser-check.cjs does, then a wait for the clock to start.
async function clearPreflight(page) {
  await page.getByRole("button", { name: "Play test tone" }).click();
  await page.getByRole("button", { name: "I heard it" }).click();
  await page
    .waitForFunction(
      () => !document.querySelector("#audio-check-join").disabled,
    )
    .catch(async (error) => {
      throw new Error(
        `Media preflight blocked: ${await page.locator("#audio-check-status").textContent()}`,
        { cause: error },
      );
    });
  await page.click("#audio-check-join");
  await page.waitForFunction(
    () => document.querySelector("#timer").textContent !== "00:00",
  );
}

test("Cmd+Enter and Ctrl+Enter in the editor run the tests without typing a newline", async (t) => {
  const ctx = await interviewPage(t);
  if (!ctx) return;
  const { page, errors } = ctx;
  try {
    const code = await page.locator("#editor").inputValue();
    // Both chords on every platform, since the handler takes either modifier.
    for (const [index, chord] of ["Meta+Enter", "Control+Enter"].entries()) {
      await runButtonReady(page);
      await page.keyboard.press(chord);
      assert.equal(await runs(page), index + 1, `${chord} ran no tests`);
      assert.equal(
        await enterDefaultPrevented(page),
        true,
        `${chord} was left to the browser's default`,
      );
    }
    assert.equal(await page.locator("#editor").inputValue(), code);
    assert.deepEqual(
      errors,
      [],
      `the shortcut threw in the browser: ${errors[0]}`,
    );
  } finally {
    await page.close();
  }
});

test("the shortcut still runs the tests after a click on the problem takes focus from the editor", async (t) => {
  const ctx = await interviewPage(t);
  if (!ctx) return;
  const { page, errors } = ctx;
  try {
    await page.click("#problem-panel");
    assert.notEqual(
      await page.evaluate(() => document.activeElement?.id),
      "editor",
    );
    await runButtonReady(page);
    await page.keyboard.press("ControlOrMeta+Enter");
    assert.equal(await runs(page), 1);
    assert.deepEqual(
      errors,
      [],
      `the shortcut threw in the browser: ${errors[0]}`,
    );
  } finally {
    await page.close();
  }
});

test("the shortcut does nothing while the media preflight is still up", async (t) => {
  const ctx = await interviewPage(t, { started: false });
  if (!ctx) return;
  const { page, errors } = ctx;
  try {
    // Enabled as far as the button knows, so what refuses the press below is
    // the shortcut's own check.
    await runButtonReady(page);
    await page.keyboard.press("ControlOrMeta+Enter");
    assert.equal(await runs(page), 0);
    // Taken, so the press got past the target and modifier checks and was
    // refused by the preflight one.
    assert.equal(await enterDefaultPrevented(page), true);
    assert.deepEqual(
      errors,
      [],
      `the shortcut threw in the browser: ${errors[0]}`,
    );
  } finally {
    await page.close();
  }
});

test("the shortcut does nothing once the interview has ended", async (t) => {
  const ctx = await interviewPage(t);
  if (!ctx) return;
  const { page, errors } = ctx;
  try {
    const code = await page.locator("#editor").inputValue();
    // Focus back in the editor is the time-up case: the candidate was still
    // typing when the clock ran out. Asserted, because a press that reached
    // no listener would pass this test without testing anything.
    await page.click("#end");
    await page.focus("#editor");
    assert.equal(
      await page.evaluate(() => document.activeElement?.id),
      "editor",
    );
    await page.keyboard.press("ControlOrMeta+Enter");
    assert.equal(await runs(page), 0);
    // Taken even with nothing to run, so it cannot type into the editor.
    assert.equal(await enterDefaultPrevented(page), true);
    assert.equal(await page.locator("#editor").inputValue(), code);
    assert.deepEqual(
      errors,
      [],
      `the shortcut threw in the browser: ${errors[0]}`,
    );
  } finally {
    await page.close();
  }
});

test("a second press while a run is in flight starts no second run", async (t) => {
  const ctx = await interviewPage(t);
  if (!ctx) return;
  const { page, errors } = ctx;
  try {
    await runButtonReady(page);
    // Both presses in one task, so the first run cannot end between them. A
    // JavaScript run finishes in milliseconds, and two presses sent from here
    // could each find the button enabled again.
    await page.evaluate(() => {
      const editor = document.querySelector("#editor");
      for (let press = 0; press < 2; press++)
        editor.dispatchEvent(
          new KeyboardEvent("keydown", {
            key: "Enter",
            ctrlKey: true,
            bubbles: true,
            cancelable: true,
          }),
        );
    });
    await runButtonReady(page);
    assert.equal(await runs(page), 1);
    assert.deepEqual(
      errors,
      [],
      `the shortcut threw in the browser: ${errors[0]}`,
    );
  } finally {
    await page.close();
  }
});

test("the shortcut starts no run while the interview is paused", async (t) => {
  const ctx = await interviewPage(t, { started: false, media: true });
  if (!ctx) return;
  const { page, errors } = ctx;
  try {
    await clearPreflight(page);
    await page.click("#pause");
    // Paused and still live. A pause that ended the interview shows the ending
    // overlay, then the report in its place 300 ms later, and the press would
    // be refused as ended, passing for the wrong reason.
    assert.equal(await page.locator("#pause").textContent(), "Resume");
    assert.equal(await page.locator("#ending-overlay").isHidden(), true);
    assert.equal(await page.locator("#report-modal").isHidden(), true);
    await page.keyboard.press("ControlOrMeta+Enter");
    assert.equal(await runs(page), 0);
    // Asserted, because a press that reached no listener would pass this
    // test without testing anything.
    assert.equal(await enterDefaultPrevented(page), true);
    assert.deepEqual(
      errors,
      [],
      `the shortcut threw in the browser: ${errors[0]}`,
    );
  } finally {
    await page.close();
  }
});
