import test from "node:test";
import assert from "node:assert/strict";
import { evaluate, MAX_EXPRESSION_LENGTH } from "../../web/calculator.js";

test("arithmetic follows ordinary precedence and associativity", () => {
  for (const [text, value] of [
    ["1 + 2 * 3", 7],
    ["(1 + 2) * 3", 9],
    ["2 ^ 3 ^ 2", 512],
    ["2 ** 10", 1024],
    ["-2 ^ 2", -4],
    ["2 ^ -1", 0.5],
    ["17 // 5", 3],
    ["17 % 5", 2],
    ["1e3 / 4", 250],
    [".5 + .25", 0.75],
    ["sqrt(16) + abs(-3)", 7],
    ["log(1000)", 3],
    ["round(pi * 100) / 100", 3.14],
  ])
    assert.equal(evaluate(text), value, text);
});

test("bad input names its problem and never runs code", () => {
  for (const text of [
    "",
    "1 +",
    "(1 + 2",
    "alert(1)",
    "1 / 0",
    "sqrt 4",
    "2 $ 3",
    "constructor",
    "1".repeat(MAX_EXPRESSION_LENGTH + 1),
  ])
    assert.throws(() => evaluate(text), Error, text);
});
