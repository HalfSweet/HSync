/** Whether this page is the data stream's own window (`?view=stream`) rather than the main one. */
export const inStreamWindow =
  typeof location !== 'undefined' && new URLSearchParams(location.search).get('view') === 'stream'
