## Clikd: Clip. Clikd. Crew.

### Complete Strategy Summary

### 🎯 Core Vision

"Clip. Clikd. Crew."

Eine Gaming-Social Platform, die den natürlichen Flow von Gaming-Content zu gemeinsamen Gaming-Sessions ermöglicht. Fokussiert auf das Wesentliche: 60fps Clips teilen, Teammates finden, zusammen zocken.

Der Slogan erklärt:

- **Clip**: Teile deinen Epic Moment (60fps, direkt aus dem Game)
- **Clikd**: Der Magic Moment — ihr habt "geklickt", connected, gematcht
- **Crew**: Jetzt seid ihr eine Crew (Team + Raum in einem), ready to game

### 🎨 Marketing & Messaging

#### Core Marketing Message

**Hauptzeile:**
- Turn clips, drops & friends into crews

**Unterzeile:**
- Clikd is the gaming network where moments lead to matches and matches to memories.

**Warum diese Message funktioniert:**
- Zeigt den kompletten User Flow (Clips → Drops → Friends → Crews)
- "Crews" als Highlight = unser Unique Begriff
- Alliteration (Moments/Matches/Memories) macht es merkbar
- Emotional statt feature-fokussiert
- "Gaming Network" positioniert uns klar

### 🚀 Die drei Säulen

#### 1) Share Clips (Content Creation)

see @.serena/memories/moment.md

#### 2) Find Crews (Team Discovery)

**Drop System** — Schnelles Team-Finding für externe Games:
- "Need Drop" Posts erstellen
- "Drop In" zu bestehenden Teams
- "Quick Drop" für Instant Matching
- Drop Zone für alle aktiven Requests
- Friend-to-Crew Pipeline für spontanes Gaming
- Skill-Verification über Clips & Stats
- Smart Matching basierend auf Playstyle

**Gaming Profile Integration:**
- Steam/Riot/Battle.net Profile verknüpfen
- Automatischer Stats Import (Rank, K/D, Playtime)
- Verified Skill Level anzeigen

**Easy Game Connect:**
- In-Game Namen teilen (formatiert zum Copy-Paste)
- Server Info/Lobby Codes automatisch formatieren
- Platform-Verknüpfung für nahtloses Adden

#### 3) Game Together (Communication)

**Crews** — Team & Raum in einem:
- 2–5 Spieler (erweiterbar mit Clikd+)
- Voice/Video/Chat/Screen Share
- Crew Recording für Auto-Clips
- Temporäre, fokussierte Sessions
- Auto-Cleanup nach Gaming

### 📱 Platform-Strategie

#### Priorität 1: Native Mobile Apps

**iOS (Swift) & Android (Kotlin):**
- Share Extensions für Direct Game Sharing
- 60fps Video Support
- Push Notifications
- Browse & Share Content unterwegs

#### Priorität 2: Desktop App (Tauri)

**Windows/Mac/Linux:**
- System Tray Integration
- Global Hotkeys für Instant Clips
- Native Performance
- Create & Play am Gaming PC

#### Priorität 3: Landing Page

**Marketing & Account Management:**
- Produkt-Präsentation & Features
- Download-Buttons für alle Platforms
- Basic Account Management
- Support & Documentation

### 💰 Monetization: Subscription Tiers

#### Clikd+ (€2.99/Monat)
- 30s Clips (statt 15s)
- 5-Person Crews
- HD Voice Quality
- Priority Matching

#### Clikd+ Pro (€7.99/Monat)
- Unlimited Clip Length
- Crew Recording
- Auto-Highlights
- Custom Themes

#### Clikd+ Crew (€14.99/Monat)
- 10-Person Crews
- Multiple Screen Shares
- Custom Crew URLs
- Creator Revenue Share

### 🏗️ Technische Architektur

#### 5 Core Services (MVP)

1. **Roster** - **User Service** — Profile mit Gaming Integration
2. **Moment** - **Content Service** — 60fps Video Processing
3. **Drop Service** — Team-Finding für externe Games
4. **Crew Service** — Voice/Video/Chat/Sessions
5. **Payment Service** — Stripe Subscriptions

#### Tech Stack

- **Backend**: Rust + Axum
- **Mobile**: Swift (iOS), Kotlin (Android)
- **Desktop**: Tauri (Rust + Web View)
- **Database**: Neon PostgreSQL
- **Real-time**: WebRTC (str0m)
- **Media**: Cloudflare R2
- **Auth**: Kanidm

### 📊 MVP Projekt-Prioritäten

#### P0 — Core MVP
- Infrastructure & Authentication
- Simple Payments Service
- Mobile Apps Foundation
- Content Feed & Discovery
- Drop Service
- Crew Service
- Profile & Gaming Integration

#### P1 — Enhancement
- Frontend Polish (Mobile-First)
- Performance Optimization

#### P2 — Launch
- Beta Testing
- Public Launch

### 🎮 User Journey

#### Path 1: Drop → Crew

1. Erstelle/Browse Drop Request ("Need 2 for Valorant")
2. Players "drop in" zum Team
3. Bei vollem Team: Auto-Start der Crew
4. Instant Voice/Video/Chat verfügbar
5. Share In-Game Namen & Server Info
6. Play together!

#### Path 2: Friend → Crew

1. Sehe Freund ist online + "Currently Playing: Valorant"
2. Click "Join Crew" Button
3. Instant in der Crew
4. Voice/Video direkt verfügbar
5. Game Info bereits im Chat (auto-populated)
6. Viral: Crew Clips mit Freunden teilen

### 📈 Success Metrics

#### User Acquisition
- 1,000 Beta Users (Month 2)
- 10,000 Active Users (Month 4)
- 100,000 Active Users (Month 8)

#### Engagement
- Friend-to-Crew Conversion > 50%
- Drop Fill Rate > 80%
- Drop-to-Crew Success > 90%
- 60fps Clip Upload Success > 95%
- Crew Creation < 3s

#### Retention
- D7 Retention > 70%
- Friend Retention > 80%
- Crew Play-Again Rate > 70%

#### Revenue
- 15% Paid Conversion
- Monthly Churn < 10%
- $50k MRR (Month 8)

### 🎯 Value Proposition

#### For Gamers
- Simple Crew Creation statt komplexer Server
- Zentrales Drop System mit Skill-Verification
- One-Click Clip Sharing in 60fps
- Friend-to-Crew Pipeline für spontanes Gaming

#### Core Differentiators
- Mobile-First für Gen Z Gamer
- 60fps als Standard überall
- Gaming-Specific Features
- Performance-Focused mit Desktop App
- Simple & fokussiert

### 🛠️ Development Roadmap

#### Phase 1: Foundation (Aug–Sep 2025)
- Infrastructure & Auth (Kanidm)
- Mobile Apps Setup
- Simple Payments
- User Service

#### Phase 2: Core Features (Okt–Nov 2025)
- Content Feed (60fps)
- Drop System
- Profile & Friends
- Gaming Integration

#### Phase 3: Communication (Dez 2025–Jan 2026)
- Crews (Team + Raum)
- Voice/Video/Chat
- Screen Sharing
- Crew Recording

#### Phase 4: Launch (Feb–Mär 2026)
- Beta Testing
- Performance Optimization
- Marketing Campaign
- Public Launch

### 💡 Market Insights

**Gaming Behavior:**
- 75% der Discord User nutzen Desktop Apps
- 87% der PC Gamer bevorzugen Desktop über Browser
- 60fps ist kritisch — Gamer merken jeden Frame Drop
- Installation ist kein Hindernis bei guter Performance

**Social Gaming:**
- Friend-based Gaming hat höhere Retention als Random Matching
- Mobile für Browse, Desktop für Create
- Gen Z nutzt Gaming als primäre soziale Interaktion

### 🎮 Clikd Terminology

#### **Drop** (Team-Finding)
**Definition:** Eine Team-Anfrage für externe Games

**Varianten:**
- **"Need Drop"** = Suche Teammates ("Need drop for Valorant Ranked")
- **"Drop In"** = Einem Team beitreten ("Drop in for a quick game?")
- **"Quick Drop"** = Instant Match mit Auto-Matching
- **"Drop Zone"** = Alle aktiven Team-Anfragen
- **"Drop Request"** = Spezifische Team-Anfrage

**Technisch:**
- Service: Drop Service
- API: `/api/drop/*`
- Status: "Looking for Drop: 2/5 Players"

#### **Crew** (Gaming-Session)
**Definition:** Temporäre Gaming-Session mit integrierter Kommunikation

**Features:**
- Team + Raum in einem (2-5 Spieler, erweiterbar)
- Voice/Video/Chat/Screen Share integriert
- Automatisches Cleanup nach 2h Inaktivität
- Session Recording für Highlights

**Flow:**
1. Drop füllt sich → Crew startet automatisch
2. Alle Spieler landen instant im Voice Chat
3. Game-Info wird geteilt
4. Nach dem Gaming: Crew löst sich auf oder "Play Again"

#### **Clip** (Content)
**Definition:** 60fps Gaming-Moment

**Specs:**
- 15-30s Standard (erweiterbar mit Clikd+)
- Immer 60fps für maximale Qualität
- Auto-Enhancement & Game Detection
- Direkt vom Mobile/PC teilbar

#### **Clikd** (Connection)
**Definition:** Der perfekte Match-Moment

**Verwendung:**
- "That clip clikd!" = Clip ging viral
- "We clikd!" = Team passt perfekt
- "That crew really clikd!" = Session war epic

### 📖 Gaming Flow

```
Clip (Content Creation)
    ↓
Drop (Team-Finding)
    ↓
Crew (Gaming Session)
    ↓
Clip (Share Highlights)
```

### 🚀 Mission Statement

Clikd ist die Platform, wo Gamer ihre besten Momente teilen, die richtigen Teammates finden und sofort zusammen spielen können. Simple, focused, performance-optimized.

**"Clip. Clikd. Crew."** — Der perfekte Gaming Flow in drei Worten.

**"Where moments lead to matches and matches to memories."**
