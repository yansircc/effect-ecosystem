import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const debug = require("debug");
export const log = debug("node:fs");
