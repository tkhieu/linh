import { describe, expect, it } from 'vitest'
import { promptSize } from './size'

describe('promptSize', () => {
  it.each([1, 2, 9])('fits all content for %i targets', (count) => {
    expect(promptSize(240 + count * 60, 1000, count * 60)).toEqual({
      width: 360, height: 240 + count * 60, maxListHeight: null,
    })
  })

  it('clamps to the monitor work area and limits only the target list', () => {
    expect(promptSize(780, 600, 540)).toEqual({
      width: 360, height: 552, maxListHeight: 312,
    })
  })
})
