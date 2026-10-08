# GDPR for the optional score sharing: research report

Date: 2026-10-08
Status: research notes, not legal advice. Claims not confirmed from a fetched page are marked [unverified].

## Scope and method

Feature under study: after the user clicks "Share" and sees a preview, the app sends an HTTPS POST to a Cloudflare Worker on the maintainer's domain, which stores in Cloudflare D1: hardware model names (CPU, GPU, disk), RAM rounded to GB, Windows build, app version, benchmark scores, date rounded to the day. Options: (a) nothing else; (b) also a random per-install UUID; (c) also an HMAC of the IP address (secret key, deleted after 30 days). Medians per model are published as a public JSON file. Alternative: users post a JSON in a public GitHub issue.

Read and used: gdpr-info.eu (Art. 2, 4, 6, 11, 13, 28, 30, 33, 37, Recital 26), EUR-Lex (Breyer), Cloudflare DPA, Cloudflare Trust Hub GDPR page, Cloudflare privacy policy, GitHub privacy statement, Blender Open Data "about" page, Homebrew analytics docs, VS Code FAQ, EDPB consultation page.

Not readable through the fetch tool: the CJEU judgment in C-413/23 P (curia redirect and press release PDF gave no text), WP216 (PDF not decodable), the Data Privacy Framework list, the EDPB final pseudonymisation guidelines, the Garante website. The SRB analysis below rests on secondary commentary (Goodwin, Data Protection Report, Freevacy).

## 1. Does GDPR apply to a private individual running the server? Household exemption? Controller?

- Art. 2(2)(c) excludes processing "by a natural person in the course of a purely personal or household activity" (verified text).
- The exemption looks at the activity, not at the person. A service that collects hardware and benchmark data from third parties, stores it in a database and publishes aggregates is not a purely personal activity. The CJEU reads the exemption narrowly: Lindqvist (C-101/01) held that publishing personal data on a website is outside it; Ryneš (C-212/13) held that CCTV covering public space is not "purely" personal (both as reported by secondary commentary, Hogan Lovells and Hunton; both decided under Directive 95/46). That the GDPR carries the exemption over unchanged is commonly stated [unverified in a fetched GDPR case-law source].
- Being non-commercial does not switch the GDPR off. The GDPR has no revenue or size threshold for applicability.
- Controller: Art. 4(7) defines the controller as the body that "determines the purposes and means of the processing". He alone designs the Worker, the D1 schema, retention and publication, so he is the controller. Cloudflare is a processor for the content of the Worker (Cloudflare privacy policy, section 6: "Cloudflare is a data processor for any of the content provided by Customers"), and a controller for its own website and account data (same section: "Cloudflare is a data controller for the personal information collected from all categories of data subjects").
- Italy: no Italian carve-out for small or non-commercial controllers was found. No Garante decision or guidance on open-source projects or the household exemption was found [unverified: absence of evidence, not proof].
- Territorial scope: he is resident in Italy, so the GDPR applies in any case. Art. 3 text not fetched [unverified].

## 2. Is an IP address seen but not stored personal data? Does in-memory rate limiting count?

- Art. 4(1): personal data is information relating to an identified or identifiable person. Recital 26: identifiability is judged by means "reasonably likely to be used ... either by the controller or by another person", considering cost, time and available technology (verified).
- Breyer (C-582/14, 19 Oct 2016), operative point as reported in the EUR-Lex text: a dynamic IP address is personal data for a provider that "has the legal means which enable it to identify the data subject" with the help of the internet service provider's data (paras 47–49 as reported by the fetch). Legal routes exist, for example in cyber-attack cases (para 47). A conservative reading treats an IP reaching the Worker as personal data.
- Transient processing: Art. 4(2) defines processing broadly (collection, consultation, use and other operations). Holding an IP in memory only to count requests is still processing [the exact Art. 4(2) text was not fetched: unverified]. Its short duration lowers the risk and the retention burden (Art. 5(1)(e) [unverified text]) but does not take it outside the GDPR.
- Natural legal basis for IP-based rate limiting: Art. 6(1)(f), legitimate interest in preventing abuse (Art. 6(1)(f) text verified).
- Cloudflare itself processes IPs at the edge (as processor for the customer, as controller for its own logs). Workers Logs retention is listed as up to 7 days, and whether IPs appear in invocation logs was not confirmed [unverified].

## 3. Options (a), (b), (c): anonymous, pseudonymous or personal? What does the 2025 SRB ruling say?

Option (a): hardware model names, RAM in GB, build, app version, scores, date rounded to the day.
- Anonymous data is outside the GDPR (Recital 26: "information which does not relate to an identified or identifiable natural person ... rendered anonymous in such a manner that the data subject is not or no longer identifiable").
- A row is not anonymous by label. A rare hardware combination with an exact score and build can single out one person, especially in a small population. The test is a case-by-case judgement that must be documented. Anonymity is more defensible when no combination is unique in the published or stored set. The WP29 criteria (singling out, linkability, inference) in Opinion 05/2014 are the usual yardstick [unverified: WP216 text not read].

Option (b): random per-install UUID.
- A persistent identifier that links records is a pseudonym. Art. 4(5) defines pseudonymisation, and Recital 26 says pseudonymised data "should be considered to be information on an identifiable natural person". Option (b) is therefore personal data, and in the controller's hands it stays personal even if the public JSON is aggregated.

Option (c): HMAC of the IP with a secret key.
- The controller holds the key, so he has the means to recompute the mapping. This is keyed pseudonymisation, and personal data under Art. 4(5) and Recital 26. The IPv4 space is small, so without the key the hash is easy to invert; with the key he can do it, which makes it personal. Deleting the hash after 30 days limits retention (Art. 5(1)(e) [unverified text]) but does not change its status during those 30 days. The raw IP is also processed momentarily before hashing.

CJEU C-413/23 P, EDPS v SRB (judgment reported 4 September 2025). From secondary sources only:
- Pseudonymised data is not automatically personal data for every holder. Whether it is depends on a context-specific assessment of the means reasonably likely to be used to identify a person (Goodwin; Data Protection Report; Freevacy).
- For a recipient that cannot re-identify the data subjects, the data may fall outside the definition. The General Court had held that; the CJEU ruled on the EDPS appeal in the SRB's favour on this point.
- Transparency duties stay with the disclosing controller. The controller assesses at collection whether the data is personal in its own hands, and the recipient's position does not change its duty to inform about recipients (one secondary summary cites paras 77, 80 and 114 [unverified against the judgment text]).
- Application here: he is the collecting controller and holds the link (the UUID in (b), the key in (c)). For (b) and (c) the data is personal in his hands, whatever the public recipient can do. The SRB reasoning could support treating published aggregates as non-personal if no reasonably likely re-identification exists, but that is for him to document.
- EDPB Guidelines 01/2025 on pseudonymisation: the consultation draft (Jan 2025) is reported to take a broader view, that pseudonymised data stays personal when it could be combined by means reasonably likely to be used by the controller or any other person (Lewis Silkin reading). The commentary says the draft diverges from the CJEU approach. The final version was not read; its adoption and text are [unverified].

## 4. Minimum concrete obligations if GDPR applies

- Privacy notice, Art. 13 (contents verified). Given at collection, so in-app before the Share button is confirmed. It must give: controller identity and contact details; purposes and legal basis; legitimate interests if relied on; recipients (Cloudflare; public publication of aggregates); transfers to the US and the safeguard used; retention period; rights (access, rectification, erasure, restriction, objection, portability); right to withdraw consent; right to complain to the Garante; whether providing data is mandatory (here: sharing is voluntary, with the consequence that nothing is shared). Language: Italian and English recommended [Art. 12 not fetched: unverified].
- Legal basis.
  - Consent, Art. 6(1)(a) (text verified), fits the explicit "Share" click after a preview. It must be specific and withdrawable (Art. 7(3) [unverified text]). Withdrawal requires a way to delete the user's data, which is simplest with option (b) plus a delete token.
  - Legitimate interest, Art. 6(1)(f), needs a documented balancing test. EDPB Guidelines 1/2024 (draft, consultation closed 20 Nov 2024) stress a restrictive reading. Final status [unverified].
  - Suggested split: consent for the sharing; legitimate interest for IP handling in rate limiting.
- Records of processing, Art. 30. The Art. 30(5) exemption for organisations under 250 persons applies only if the processing is occasional, carries no risk to data subjects, and involves no special categories or criminal data (text verified). A continuously running database is probably not "occasional", so the exemption is likely lost. A short record (purposes, categories, recipients, transfers, retention, security measures) is cheap; the exact Art. 30(1) list was not fetched [unverified].
- Processor contract, Art. 28. Art. 28(3) requires a contract with documented instructions (point a) and audit/information duties (point h) (verified). Cloudflare's DPA is incorporated by reference into the Self-Serve Subscription Agreement and per Cloudflare "these customers need to take no action" (Trust Hub). The DPA includes the EU SCCs (Module 2, clause 6.2), sub-processor rules (clauses 4.3–4.4) and a DPF commitment (clause 6.4). It was not checked clause by clause against all of Art. 28(3) [unverified].
- International transfers. Cloudflare says it complies with the EU-US Data Privacy Framework (DPA clause 6.4; privacy policy section 7; Trust Hub). If certification lapses, the SCCs apply. The DPF list entry could not be read, so the current listing is [unverified]. Cloudflare says it stores data primarily in the US and the EEA. Whether D1 data can be pinned to the EU was not confirmed [unverified].
- Data subject rights. Art. 11(2): if the controller can demonstrate it cannot identify the person, Arts. 15–20 do not apply unless the person supplies extra information (text verified). That fits option (a). With (b) or (c), access and erasure need a handle: the install UUID or a delete token that the app shows at submission. Erasure is Art. 17 [text not fetched].
- DPO, Art. 37(1). Mandatory only for public authorities, regular and systematic large-scale monitoring, or large-scale special-category data (verified). None applies. No DPO needed.
- Breach notification. Art. 33(1): notify the Garante without undue delay and where feasible within 72 hours, unless the breach is unlikely to risk rights and freedoms. Art. 33(5): document every breach, even unnotified ones (verified). Art. 33(2): processors notify the controller without undue delay (verified), so Cloudflare must tell him. Notice to individuals (Art. 34) [unverified].
- DPIA (Art. 35): probably not needed for a small dataset without special categories [unverified: not fetched].
- Data protection by design and minimisation (Art. 25, Art. 5(1)(c)) [unverified text]: a strong argument for not storing the IP at all, and for keeping the field list short.

## 5. The GitHub issue alternative

- The user posts a JSON in a public issue. His GitHub account name is personal data, and the hardware fields are personal data if they can be linked to him. Posting to a public platform is outside the household exemption under the narrow CJEU reading (Lindqvist, see section 1).
- GitHub's statement says that content shared in a collaborative context "may become publicly accessible" and that users control what is made public. It does not set obligations for users who post data about other people (verified). For a personal account GitHub acts as controller of its own service data; GitHub is a processor only where an employer or school supplies the account (verified).
- If the maintainer's Action parses the issues and aggregates them, he becomes the controller of that processing. He determines purpose and means. Indirect collection triggers the notice duty in Art. 14 [text not fetched: unverified], with an exception where it is disproportionate.
- Mitigations: parse only the fixed JSON fields, drop the GitHub username before any output, store no issue bodies, publish aggregates only above a minimum count, and rely on GitHub's own deletion route for individual posts.
- Privacy-wise the issue route is weaker for the user: the data stays public, indexed and persistent, and deletion depends on GitHub's tools and the user's own action.

## 6. Italy-specific points

- The Codice privacy (d.lgs. 196/2003) as amended by d.lgs. 101/2018 adapts national law to the GDPR. The GDPR applies directly. The Garante is the Italian supervisory authority (general knowledge [unverified: Garante page not fetched]).
- Art. 2-quaterdecies of the Codice (as amended) covers the designation of persons who act under the controller's authority. It matters only if other people help operate the service (search snippet of the text, not fetched).
- d.lgs. 101/2018 directs the Garante to set simplified measures for micro, small and medium enterprises in its guidelines (secondary source). The exact article is [unverified]. The Garante's 2007 SME guide predates the GDPR and should not be relied on.
- Age of consent for information society services in Italy is 14 under Art. 2-quinquies of the Codice [unverified: general knowledge; not fetched]. Relevant only if the feature could be used by minors.
- No Garante guidance on open-source projects or the household exemption was found [unverified: absence not confirmed on the Garante site].

## 7. How comparable projects handle it

- Blender Open Data (opendata.blender.org/about, fetched): "All data is kept anonymous by default"; a display name is opt-in; sharing is optional; results are published as public-domain JSON and CSV. It describes its privacy approach as "privacy-conscious" and names no retention period or deletion process on that page. The footer links to blender.org/privacy-policy (not read) [unverified].
- OpenBenchmarking.org (Phoronix Test Suite): upload is opt-in per run. A third-party reproduction says the platform keeps public and private storage including system logs. Privacy policy and retention were not confirmed [unverified].
- Homebrew analytics (docs.brew.sh/Analytics, fetched): sent after a first-run notice. The payload "does not contain a user identifier or an IP-address field". Retention is 365 days in InfluxDB. Opt-out with `brew analytics off` or `HOMEBREW_NO_ANALYTICS=1`. This is the closest model to option (a).
- KDE telemetry (2017 mailing-list draft policy, via search summary): opt-in and off by default; bars unique device, installation or user IDs; keeps network data (such as IP addresses) separate; states that deleting one user's data is effectively impossible. The current policy at community.kde.org was not fetched [unverified current wording].
- VS Code (code.visualstudio.com FAQ, fetched): usage and crash data sent to Microsoft by default; opt-out with `telemetry.telemetryLevel` set to `off`. The FAQ does not describe IP or ID handling [unverified].
- Fedora: the telemetry proposal is reported to cover only the GNOME variant (secondary, Privacy Guides forum) [unverified].

Pattern: the projects with no persistent identifier and no IP field (Homebrew, Blender) are the closest analogues to option (a). KDE's policy is the strongest public statement that avoiding unique IDs is the way to stay outside personal data. None of them publishes a full GDPR analysis that we could read.

## Practical summary

| Option | Personal data? | Main obligations | Risk |
|---|---|---|---|
| (a) only the record fields; IP used transiently for rate limiting, not stored | Records: probably not personal if no record is unique and this is documented. IP during rate limiting: treat as personal (conservative). | Notice covering the IP processing; legal basis for it (Art. 6(1)(f)); Cloudflare DPA (already incorporated); short log retention; short record of processing recommended | Low to moderate |
| (b) (a) plus random install UUID | Yes: pseudonymised, personal in his hands (Recital 26, Art. 4(5)) | Everything in (a), plus consent for sharing; a delete route keyed on the UUID or a delete token; record of processing likely required; Art. 11(2) no longer helps | Moderate |
| (c) (a) plus HMAC of IP, deleted after 30 days | Yes: keyed pseudonym, personal while he holds the key | Everything in (b), plus key management, a deletion job, and notice of the momentary IP processing before hashing | Moderate to high (more processing, more to explain) |
| GitHub issue posted by the user | Yes if linked to the account (username). Hardware fields alone: low. | User's own act; if the maintainer's Action aggregates: controller duties (Art. 14 notice, minimisation, usernames stripped); GitHub handles hosting | Low if the Action strips identities; the user needs a GitHub account |

Observations, not legal advice:
- The lowest-risk design is (a) with in-memory rate limiting keyed on the IP, no IP written to D1 or to logs, and a documented uniqueness check on published aggregates.
- "One result per install per model" can be approximated without a stored UUID, for example by accepting duplicates and letting medians absorb them, which the aggregation already does.
- If a UUID is kept, the feature needs a delete route, and the privacy notice must say so.
- The one-line caveat: this is research, not legal advice; a lawyer or the Garante's guidance should be consulted before launch.

## Items marked [unverified]

- CJEU C-413/23 P: paragraph numbers (77, 80, 114) and holdings: secondary sources only, judgment text not read.
- EDPB Guidelines 01/2025 on pseudonymisation: final adoption and text; the draft position is from secondary commentary.
- EDPB Guidelines 1/2024 (legitimate interest): final status.
- WP216 (Opinion 05/2014): hashing and singling-out criteria, not read.
- Art. 2(2)(c) application to open-source servers under the GDPR, beyond the CJEU narrow-reading cases (secondary sources; Directive 95/46 cases).
- Art. 3, Art. 4(2), Art. 5(1)(c) and (e), Art. 7(3), Art. 12, Art. 14, Art. 17, Art. 25, Art. 30(1), Art. 34, Art. 35: text not fetched.
- Cloudflare DPA: full clause-by-clause check against Art. 28(3).
- Cloudflare DPF listing: the official list could not be read; certification is claimed by Cloudflare.
- Cloudflare: whether D1 data location can be pinned to the EU; whether IP addresses appear in Workers logs; the Workers log retention figure (7 days, from a search snippet).
- Garante: any decision or guidance on open-source projects or the household exemption; the simplified-measures article in d.lgs. 101/2018; the age-14 rule of Art. 2-quinquies.
- Blender privacy policy text; OpenBenchmarking privacy policy and retention.
- KDE current telemetry policy (only the 2017 draft was read).
- VS Code: IP and ID handling.
- Fedora telemetry scope (secondary source only).
- Whether GitHub is controller for public content of personal accounts.

## Sources

Legal texts
- https://gdpr-info.eu/art-2-gdpr/
- https://gdpr-info.eu/recitals/no-26/
- https://gdpr-info.eu/art-4-gdpr/
- https://gdpr-info.eu/art-6-gdpr/
- https://gdpr-info.eu/art-11-gdpr/
- https://gdpr-info.eu/art-13-gdpr/
- https://gdpr-info.eu/art-28-gdpr/
- https://gdpr-info.eu/art-30-gdpr/
- https://gdpr-info.eu/art-33-gdpr/
- https://gdpr-info.eu/art-37-gdpr/
- https://eur-lex.europa.eu/legal-content/EN/TXT/?uri=CELEX:62014CJ0582 (Breyer, C-582/14)
- https://curia.europa.eu/jcms/upload/docs/application/pdf/2025-09/cp250107en.pdf (SRB press release, not readable)

CJEU commentary (secondary)
- https://www.goodwinlaw.com/en/insights/publications/2025/09/alerts-technology-dpc-personal-data-or-not
- https://www.dataprotectionreport.com/2025/09/pseudonymised-data-could-fall-outside-data-protection-law-introducing-the-means-reasonably-likely-assessment/
- https://www.freevacy.com/news/cjeu/cjeu-ruling-clarifies-pseudonymised-data-is-not-always-personal-data/6693
- https://www.lewissilkin.com/insights/2025/02/21/pseudonymisation-the-edpb-guidelines-and-the-cjeu-advocate-generals-opinion-in-102k16i
- https://www.hoganlovells.com/en/publications/cctv-cjeu-narrows-the-scope-of-the-household-exemption (Ryneš)
- https://www.hunton.com/privacy-and-information-security-law/cjeu-adopts-strict-approach-use-cctv (Ryneš)
- https://www.cms.law/en/bgr/legal-updates/are-dynamic-ip-addresses-personal-data-europe-s-highest-court-says-they-are-in-certain-circumstances (Breyer)

EDPB and Article 29 WP
- https://www.edpb.europa.eu/our-work-tools/documents/public-consultations/2025/guidelines-012025-pseudonymisation_en
- https://www.edpb.europa.eu/our-work-tools/documents/public-consultations/2024/guidelines-12024-processing-personal-data-based_en
- https://ec.europa.eu/justice/article-29/documentation/opinion-recommendation/files/2014/wp216_en.pdf (not readable)

Cloudflare
- https://www.cloudflare.com/cloudflare-customer-dpa/
- https://www.cloudflare.com/trust-hub/gdpr/
- https://www.cloudflare.com/privacypolicy/
- https://developers.cloudflare.com/workers/observability/logs/workers-logs/ (search snippet)
- https://developers.cloudflare.com/workers/runtime-apis/bindings/rate-limit/ (search snippet)
- https://www.dataprivacyframework.gov/list (not readable)

GitHub
- https://docs.github.com/en/site-policy/privacy-policies/github-privacy-statement

Comparable projects
- https://opendata.blender.org/about/
- https://docs.brew.sh/Analytics
- https://code.visualstudio.com/docs/supporting/faq
- https://mail.kde.org/pipermail/kde-community/2017q3/003806.html (2017 draft policy)
- https://community.kde.org/Policies/Telemetry_Policy (not fetched)
- https://openbenchmarking.org/features (search snippet)
- https://discussion.fedoraproject.org/t/what-data-will-be-collected-exactly-a-breakout-topic-for-the-f40-change-request-on-privacy-preserving-telemetry-for-fedora-workstation/85417/30 (secondary)

Italy
- https://www.garanteprivacy.it/ (not fetched)
- https://www.brocardi.it/codice-della-privacy/parte-i/titolo-i/capo-iv/ (search result only)
