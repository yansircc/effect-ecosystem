import cron from "node-cron"
export const job = cron.schedule("* * * * *", () => undefined)
