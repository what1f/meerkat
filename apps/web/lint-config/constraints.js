const layers = ['app', 'features', 'shell', 'scene', 'design', 'lib', 'dev'];
const allowed = {
  app: layers,
  features: ['design', 'scene', 'lib', 'features'],
  shell: ['shell', 'design', 'scene', 'lib'],
  scene: ['scene', 'design'],
  design: ['design'],
  lib: ['lib'],
  dev: ['dev', 'design', 'scene', 'shell'],
};
const zones = layers.flatMap((target) =>
  layers
    .filter((from) => !allowed[target].includes(from))
    .flatMap((from) => [
      { target: `./src/${target}`, from: `./src/${from}` },
      { target: `./lint-fixtures/${target}`, from: `./src/${from}` },
    ]),
);
// Isolate the planned feature directories before they are implemented.
const features = ['members', 'team', 'office', 'settings', 'onboarding'];
for (const target of features)
  for (const from of features)
    if (target !== from) {
      zones.push({ target: `./src/features/${target}`, from: `./src/features/${from}` });
      zones.push({
        target: `./lint-fixtures/features/${target}`,
        from: `./lint-fixtures/features/${from}`,
      });
    }
const native = 'JSXOpeningElement[name.name=/^(button|input|textarea|select|dialog|img|svg)$/]';
const subscriptions = [
  {
    selector: 'CallExpression[callee.name=/^(setTimeout|setInterval|WebSocket)$/]',
    message: 'Keep subscriptions in lib hooks.',
  },
  {
    selector: "NewExpression[callee.name='WebSocket']",
    message: 'Keep subscriptions in lib hooks.',
  },
  {
    selector:
      'CallExpression[callee.object.name=/^(window|globalThis)$/][callee.property.name=/^(setTimeout|setInterval)$/]',
    message: 'Keep timers in lib hooks.',
  },
];
const path = "JSXOpeningElement[name.name='path']";

export { zones, native, subscriptions, path };
