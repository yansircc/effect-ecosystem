import { createRequire } from "node:module";
import nodeModule from "node:module";

const require = createRequire(import.meta.url);
const { readFileSync } = require("node:fs");
const nodeRequire = nodeModule.createRequire(import.meta.url);
const extra = nodeRequire("fs-extra");
const load = nodeRequire;
const promises = load("node:fs/promises");
export const text = String(readFileSync) + String(extra) + String(promises);
