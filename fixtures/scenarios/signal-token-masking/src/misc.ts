#!/usr/bin/env node
"use strict"
import type { Effect } from "effect"
import Effect2 = require("effect")
export const e = `HttpApiGroup`
export enum En { A = "HttpApiEndpoint", "RpcGroup" = 2 }
declare module "HttpApiGroup" { }
type T = `Rpc.make-${string}`
export { x as "RpcGroup" } from "./y"
