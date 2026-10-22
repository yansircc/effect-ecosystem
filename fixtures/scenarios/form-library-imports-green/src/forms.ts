import { Schema } from "effect"
import { useForm } from "@tanstack/react-form"
import { useForm as useReactHookForm } from "react-hook-form"
import { useFormik } from "formik"
export const formFacts = { Schema, useForm, useReactHookForm, useFormik }
