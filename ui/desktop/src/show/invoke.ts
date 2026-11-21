// The show page's calls to octopus, by the command names the app declares.

import { invoke as invokeAny, type InvokeArgs } from "@tauri-apps/api/core";
import { COMMANDS } from "../generated";

export const invoke = <T>(command: (typeof COMMANDS)[number], args?: InvokeArgs) => invokeAny<T>(command, args);
