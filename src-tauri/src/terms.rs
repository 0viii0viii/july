//! 전사 후 용어 교정.
//!
//! whisper의 `initial_prompt`는 첫 30초 윈도우를 주로 조건화해서 긴 회의에서는
//! 뒤로 갈수록 효과가 옅어진다. 게다가 힌트를 늘리면 디코딩 전체가 편향돼
//! 엉뚱한 곳이 망가진다 — 실측에서 용어를 더 넣었더니 "배포하는"이 "체포하는"이
//! 됐다. 그래서 표기 교정은 전사가 끝난 뒤에 따로 한다. 후처리는 지정한 용어에만
//! 국소적으로 작용하고 나머지 문장은 건드리지 않는다.
//!
//! 실제 회의 녹음(18분, 클라우드 인프라 주제)으로 임계값을 정했다. 그 녹음에서
//! 영문 토큰 100개 중 44개가 교정 대상이었고 오교정은 0건이었다.
//!
//! 한국어 고유명사 교정(자모 단위 근사 매칭)은 아직 없다. 같은 녹음에서 한국어
//! 고유명사는 전부 정확했고("전북은행" 11회 중 11회), 임계값을 정할 오류 표본이
//! 없었다. 검증 못 한 임계값으로 한국어를 건드리면 얻는 것보다 잃는 게 크다.

use std::collections::HashMap;

/// 클라우드 업계 표준 용어의 정답 표기.
///
/// 축약형을 따로 넣는 게 중요하다. `Kubernetes`만 있으면 오인식된 `Qube`를 못
/// 잡는다 — 편집거리가 너무 멀다. 사람들이 실제로 말하는 `Kube`가 있어야 걸린다.
const CATALOG: &[&str] = &[
    // 가상화
    "VMware", "vCenter", "vSphere", "ESXi", "vMotion", "vSAN", "OVF", "OVA",
    "Nutanix", "Openstack", "Hyper-V",
    // 컨테이너·오케스트레이션
    "Kubernetes", "Kube", "K8s", "Docker", "Helm", "Istio",
    // 퍼블릭 클라우드
    "AWS", "EC2", "S3", "RDS", "EKS", "ECS", "VPC", "IAM", "EBS", "ELB",
    "Lambda", "CloudFront", "Route53", "Azure", "GCP", "NCP", "NKS",
    // 운영·도구
    "Terraform", "Ansible", "Jenkins", "GitLab", "Prometheus", "Grafana",
    "CMP", "CSP", "MSP", "SaaS", "PaaS", "IaaS", "DevOps",
    // 인프라
    "GPU", "CPU", "RAM", "SSD", "NAS", "SAN", "CDN", "DNS", "NAT", "ACL",
    "VPN", "WAF", "LDAP", "SSO", "MFA",
    // 일반
    "API", "SDK", "CLI", "SLA", "PoC", "SMS", "SNS",
];

/// 퍼지 매칭에 넣을 최소 길이.
///
/// **이 값이 결정적인 가드다.** 3으로 내리면 영어 단어 `open`이 `VPN`으로 바뀐다
/// (실측). 짧은 토큰은 무관한 단어와 편집거리가 가까워서 손대면 안 된다.
const FUZZY_MIN_LEN: usize = 4;

/// 허용할 최대 편집거리. 3으로 올려도 새로 잡히는 게 하나도 없었다.
const MAX_DIST: usize = 2;

/// 편집거리를 긴 쪽 길이로 나눈 비율의 상한. 길이 가드와 함께 쓰면 여유가 있다.
const MAX_RATIO: f32 = 0.45;

/// 사용자가 쉼표나 공백으로 적어준 용어에서 라틴 문자 항목만 뽑는다.
///
/// 한국어 항목은 여기서 버린다 — 한국어 교정은 아직 구현하지 않았고, 라틴
/// 매칭에 한국어를 섞으면 편집거리가 의미를 잃는다.
fn user_latin_terms(terms: &str) -> Vec<String> {
    terms
        .split(|c: char| c == ',' || c.is_whitespace())
        .map(str::trim)
        .filter(|t| {
            !t.is_empty()
                && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                && t.chars().any(|c| c.is_ascii_alphabetic())
        })
        .map(str::to_string)
        .collect()
}

fn levenshtein(a: &[u8], b: &[u8]) -> usize {
    let (m, n) = (a.len(), b.len());
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut cur = vec![0usize; n + 1];
    for i in 1..=m {
        cur[0] = i;
        for j in 1..=n {
            let sub = prev[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(sub);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[n]
}

/// 이 토큰을 어떤 정답 표기로 바꿔야 하는지 찾는다. 바꿀 필요가 없으면 `None`.
fn canonical_for(token: &str, catalog: &[String], lookup: &HashMap<String, usize>) -> Option<String> {
    let lower = token.to_ascii_lowercase();

    // 1단계 — 표기 분열 정리. 같은 용어가 s3/S3/S3. 로 갈리면 요약 모델이 다른
    // 것으로 취급한다. 알려진 용어의 대소문자만 맞추는 것이라 오교정이 구조적으로
    // 불가능하다.
    if let Some(&i) = lookup.get(&lower) {
        return (catalog[i] != token).then(|| catalog[i].clone());
    }

    // 2단계 — 오인식 교정. whisper가 영문 용어를 음차로 뭉갠 경우다.
    if token.len() < FUZZY_MIN_LEN {
        return None;
    }
    let mut best: Option<(&str, usize)> = None;
    for term in catalog {
        if term.len() < FUZZY_MIN_LEN {
            continue;
        }
        let d = levenshtein(lower.as_bytes(), term.to_ascii_lowercase().as_bytes());
        if d > MAX_DIST || d as f32 / token.len().max(term.len()) as f32 > MAX_RATIO {
            continue;
        }
        if best.is_none_or(|(_, bd)| d < bd) {
            best = Some((term, d));
        }
    }
    best.map(|(t, _)| t.to_string())
}

/// 녹취록의 영문 용어 표기를 바로잡는다.
///
/// `user_terms`는 회의 정보 화면에서 받은 용어다. 내장 카탈로그와 합쳐지고,
/// 같은 이름이 있으면 사용자 쪽이 이긴다 — 사내 표기가 업계 표기와 다를 수 있다.
pub fn correct(text: &str, user_terms: &str) -> String {
    let mut catalog: Vec<String> = user_latin_terms(user_terms);
    let mut lookup: HashMap<String, usize> = HashMap::new();
    for (i, t) in catalog.iter().enumerate() {
        lookup.insert(t.to_ascii_lowercase(), i);
    }
    for t in CATALOG {
        // 사용자가 같은 용어를 다른 표기로 줬으면 덮어쓰지 않는다.
        lookup.entry(t.to_ascii_lowercase()).or_insert_with(|| {
            catalog.push((*t).to_string());
            catalog.len() - 1
        });
    }

    // ASCII 영숫자 구간만 훑는다. ASCII 바이트는 UTF-8 멀티바이트 시퀀스 안에
    // 나타날 수 없으므로 바이트 단위로 잘라도 한글이 깨지지 않는다.
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_alphabetic() {
            // 멀티바이트 문자는 통째로 넘긴다.
            let start = i;
            i += 1;
            while i < bytes.len() && (bytes[i] & 0xC0) == 0x80 {
                i += 1;
            }
            out.push_str(&text[start..i]);
            continue;
        }
        let start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric()) {
            i += 1;
        }
        let token = &text[start..i];
        match canonical_for(token, &catalog, &lookup) {
            Some(fixed) => out.push_str(&fixed),
            None => out.push_str(token),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 실제 회의 녹음에서 관찰된 표기 분열. 같은 용어가 세 가지로 갈렸다.
    #[test]
    fn normalizes_split_casing() {
        assert_eq!(correct("s3에 올리고 S3를 확인", ""), "S3에 올리고 S3를 확인");
        assert_eq!(correct("cmp에서 ncp로, aws도 ovf도", ""), "CMP에서 NCP로, AWS도 OVF도");
    }

    /// 같은 녹음에서 16번 연속으로 틀린 오인식. 한 줄이 오류 16개를 지운다.
    #[test]
    fn fixes_misrecognized_vmware() {
        assert_eq!(correct("vmure에 로그인했었던", ""), "VMware에 로그인했었던");
    }

    /// 축약형이 카탈로그에 있어야 잡힌다. Kubernetes만으론 거리가 멀다.
    #[test]
    fn fixes_short_form_via_catalog() {
        assert_eq!(correct("Qube 클러스터", ""), "Kube 클러스터");
    }

    /// 회귀 방지 — 최소길이를 3으로 내리면 이게 VPN으로 바뀐다.
    #[test]
    fn leaves_ordinary_english_word_alone() {
        assert_eq!(correct("open 상태로 두죠", ""), "open 상태로 두죠");
    }

    /// 한국어는 건드리지 않는다. 조사가 붙어도 마찬가지다.
    #[test]
    fn leaves_korean_alone() {
        let s = "전북은행에 후속조치 방안을 공유했습니다";
        assert_eq!(correct(s, ""), s);
    }

    /// 짧은 토큰은 퍼지 매칭에서 제외된다.
    #[test]
    fn leaves_short_tokens_alone() {
        assert_eq!(correct("v 그리고 V 얘기", ""), "v 그리고 V 얘기");
    }

    /// 사용자가 준 용어가 내장 카탈로그를 이긴다 — 사내 표기가 다를 수 있다.
    #[test]
    fn user_terms_win_over_catalog() {
        assert_eq!(correct("vmware 얘기", "VMWare"), "VMWare 얘기");
    }

    /// 사용자 용어 중 한국어 항목은 라틴 매칭에 섞이지 않는다.
    #[test]
    fn ignores_korean_user_terms() {
        assert_eq!(correct("open 합니다", "박대리, 전북은행"), "open 합니다");
    }

    #[test]
    fn preserves_text_without_latin() {
        let s = "다들 모이셨죠. 시작하겠습니다.";
        assert_eq!(correct(s, ""), s);
    }
}
