import { object, string } from "yup"
export const User = object({ name: string().required() })
