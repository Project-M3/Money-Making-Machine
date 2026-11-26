import { Corrections } from './sections/Corrections'
import { Editorial } from './sections/Editorial'
import { Footer } from './sections/Footer'
import { FrontPage } from './sections/FrontPage'
import { Install } from './sections/Install'
import { Masthead } from './sections/Masthead'
import { Roadmap } from './sections/Roadmap'
import { ShowYourWork } from './sections/ShowYourWork'
import { TryIt } from './sections/TryIt'

export function App() {
  return (
    <div class="min-h-screen bg-paper text-ink">
      <Masthead />
      <main id="top">
        <FrontPage />
        <ShowYourWork />
        <TryIt />
        <Editorial />
        <Corrections />
        <Roadmap />
        <Install />
      </main>
      <Footer />
    </div>
  )
}
