import { attach } from "./worker-core.js";
attach(
  (m) => self.postMessage(m),
  (h) => {
    self.onmessage = (e) => h(e.data);
  },
);
