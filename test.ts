import { disableThrottling, intercept, original } from ".";

intercept();

let lastDate = Date.now();
disableThrottling();
setInterval(() => {
  console.log(Date.now() - lastDate);
  lastDate = Date.now();
  // console.log(original);
});
