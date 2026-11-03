// The five steps of the first-run tour. `selector` marks the region to highlight; the Shell
// sets the data-tour attributes, and a step whose region is missing is shown without one.

// The tour id stored by `PUT /users/me/tours/{tour}`.
export const TOUR_ID = 'shell-v1';

// The steps in order.
export const TOUR_STEPS = [
  { id: 'tree', selector: '[data-tour="tree"]', title: 'The inventory tree', text: 'Every data center, site, node and sandbox you may see. Switch the view for infrastructure, workloads or models.' },
  { id: 'context', selector: '[data-tour="context"]', title: 'The context picker', text: 'Choose the data center you work in. Lists and actions follow this choice.' },
  { id: 'search', selector: '[data-tour="search"]', title: 'Search everything', text: 'Press Ctrl K (Cmd K on a Mac) to jump to a sandbox, node, model, policy or document.' },
  { id: 'actions', selector: '[data-tour="actions"]', title: 'The action bar', text: 'The actions of the page you are on stay here while the page scrolls.' },
  { id: 'dock', selector: '[data-tour="dock"]', title: 'The dock', text: 'Running tasks, live events and alerts wait for your answer here. Press d to open or close it.' },
];
