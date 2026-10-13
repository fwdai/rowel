/**
 * Each issuing country's passport cover colour, by ISO 3166-1 alpha-3 code.
 *
 * Sampled once from the cover photos Wikidata links its passport items to, and
 * corrected by hand where a sample caught gold foil, a photo's background or a
 * faded scan; the handful with no usable photo are named from what their
 * covers are known to be. A country missing here gets the neutral `FALLBACK` —
 * most of what is missing is a territory that issues no passport of its own.
 */
const COLORS: Record<string, string> = {
  AFG: '#537C90', AGO: '#121C1C', ALB: '#481F2D', AND: '#341D17', ARE: '#172748',
  ARG: '#1D2B52', ARM: '#172C49', AUS: '#0D2049', AUT: '#882C50', AZE: '#012D20',
  BEL: '#542F35', BEN: '#35604F', BFA: '#206259', BGD: '#006147', BGR: '#6C353D',
  BHR: '#9E2A31', BIH: '#1D2B52', BLR: '#002E6E', BOL: '#7A1F2B', BRA: '#1D2B52',
  BRN: '#B13C34', BWA: '#232323', CAN: '#172C49', CHE: '#DE232A', CHL: '#572C34',
  CHN: '#390C1F', CIV: '#063015', CMR: '#3F3E3B', COD: '#3A4764', COG: '#232323',
  COL: '#6B1C2A', COM: '#232C1E', CPV: '#011A6A', CRI: '#232341', CUB: '#1E3367',
  CYP: '#992C5A', CZE: '#734E5E', DEU: '#7F0D40', DJI: '#2A5199', DNK: '#6B1C2A',
  DOM: '#1F2D5A', DZA: '#083C3B', ECU: '#6B1C2A', EGY: '#1D582F', ERI: '#0E2733',
  ESP: '#8A1741', EST: '#7D274A', ETH: '#8A70A4', FIN: '#771F36', FRA: '#571927',
  FSM: '#21242A', GAB: '#305859', GBR: '#1B2233', GEO: '#622A29', GHA: '#233229',
  GIN: '#47473D', GNB: '#175C37', GRC: '#800020', GTM: '#1D2B52', HKG: '#1D2B52',
  HND: '#1D2B52', HRV: '#2D2B2E', HUN: '#4F2332', IDN: '#237D96', IND: '#070930',
  IRL: '#5E2640', IRN: '#391B20', IRQ: '#2B3562', ISL: '#2B345E', ISR: '#3D4276',
  ITA: '#A33A56', JAM: '#1D2B52', JOR: '#252E38', JPN: '#730714', KAZ: '#036C96',
  KEN: '#1D2F5E', KGZ: '#00428A', KHM: '#460607', KOR: '#1E2A4F', LAO: '#162745',
  LBN: '#10223E', LBR: '#3C3F3B', LBY: '#171A5B', LIE: '#132738', LKA: '#641C39',
  LTU: '#6E0118', LUX: '#7A1E3A', LVA: '#460E17', MAC: '#1F4A3A', MAR: '#374021',
  MCO: '#521D29', MDA: '#642535', MDG: '#571927', MDV: '#C91718', MEX: '#002A1F',
  MHL: '#1D2440', MKD: '#552432', MLT: '#781D27', MMR: '#A52A2E', MNE: '#802F30',
  MNG: '#7B2236', MOZ: '#172543', MRT: '#2E6F35', MWI: '#232323', MYS: '#9F1A1B',
  NAM: '#28443F', NER: '#348B44', NGA: '#154C38', NIC: '#1D2B52', NLD: '#612B33',
  NOR: '#B5232B', NPL: '#1B5552', NZL: '#323737', OMN: '#6B1C2A', PAK: '#284830',
  PAN: '#1B1E54', PER: '#6B1C2A', PHL: '#750B0B', POL: '#50192F', PRK: '#2E3043',
  PRT: '#3C1815', PRY: '#1D2B52', PSE: '#2B2A24', QAT: '#55403B', ROU: '#7A3C2D',
  RUS: '#A02539', RWA: '#0399BD', SAU: '#0E4331', SDN: '#1A181F', SEN: '#73493C',
  SGP: '#A4312A', SLV: '#1D2B52', SOM: '#54828E', SRB: '#531726', STP: '#3A190C',
  SVK: '#5C1133', SVN: '#5F2D38', SWE: '#800020', SWZ: '#26295A', SYC: '#8B5958',
  SYR: '#012623', TCD: '#262626', TGO: '#608A96', THA: '#512930', TJK: '#3D2A1B',
  TKM: '#11441A', TLS: '#5B1817', TUN: '#282F2F', TUR: '#6D1825', TUV: '#10A1F5',
  TWN: '#134D3E', TZA: '#212E5F', UGA: '#0371AE', UKR: '#17305D', URY: '#1D2B52',
  USA: '#1C2A4D', UZB: '#53191C', VAT: '#144940', VEN: '#6B1C2A', VNM: '#1B1E3F',
  VUT: '#2B9C50', YEM: '#100335', ZAF: '#26322A', ZMB: '#232323',
}

const FALLBACK = '#2B3240'

export const coverColor = (code: string): string => COLORS[code] ?? FALLBACK
