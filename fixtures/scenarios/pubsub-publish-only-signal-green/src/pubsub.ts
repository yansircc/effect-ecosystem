import { PubSub } from "effect"
export const publishOnly = (bus: PubSub.PubSub<string>) => PubSub.publish(bus, "x")
