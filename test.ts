import { disableThrottling, intercept, original, setPositionAbsolute } from ".";

intercept();

let lastDate = Date.now();
disableThrottling();

function loop() {
  console.log(Date.now() - lastDate);
  lastDate = Date.now();
  // console.log(original);
  setPositionAbsolute({ x: original.x, y: original.y });
  setImmediate(loop);
}

loop();
