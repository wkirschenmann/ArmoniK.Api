import fs from 'node:fs'
import { resolve } from 'pathe'
import consola from 'consola'

export function _readAndReplace(pattern: RegExp, replace: string) {
  return (file: string) => {
    const data = fs.readFileSync(resolve(file), 'utf8')

    // A pattern that matches nothing rewrites the file unchanged, and reporting success for it is
    // how a moved or renamed version tag stops being versioned without anyone hearing. Matched
    // rather than compared, because a file already at the target version is a legitimate re-run.
    if (!data.match(pattern)) {
      consola.fatal(`No match for ${pattern} in ${file}: nothing was versioned`)
      process.exit(1)
    }

    const result = data.replace(pattern, replace)

    fs.writeFileSync(resolve(file), result, 'utf8')

    consola.success(`Update ${file.split('/').pop()}`)
  }
}
