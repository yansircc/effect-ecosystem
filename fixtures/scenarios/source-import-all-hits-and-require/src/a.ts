import { z } from "zod"
import type { ZodType } from "zod/v4"
const mini = require("zod/mini")
import fs from "node:fs/promises"
import extra from "fs-extra"
import { useQuery } from "@tanstack/query-core"
export const facts = [z, mini, fs, extra, useQuery]
