import { parentPort } from "node:worker_threads";
import { attach } from "../worker-core.js";
attach((m) => parentPort.postMessage(m), (h) => parentPort.on("message", h));
