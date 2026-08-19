//! 전사 후 용어 교정.
//!
//! whisper의 `initial_prompt`는 첫 30초 윈도우를 주로 조건화해서 긴 회의에서는
//! 뒤로 갈수록 효과가 옅어진다. 게다가 힌트를 늘리면 디코딩 전체가 편향돼
//! 엉뚱한 곳이 망가진다 — 실측에서 용어를 더 넣었더니 "배포하는"이 "체포하는"이
//! 됐다. 그래서 표기 교정은 전사가 끝난 뒤에 따로 한다. 후처리는 지정한 용어에만
//! 국소적으로 작용하고 나머지 문장은 건드리지 않는다.
//!
//! **교정은 두 가지뿐이고, 둘 다 오교정이 구조적으로 불가능하다.**
//!
//! 1. 알려진 용어의 대소문자 정규화 (`s3` → `S3`)
//! 2. 확인된 오인식의 명시적 치환 (`vmure` → `VMware`)
//!
//! 근사 매칭(편집거리)은 시도했다가 버렸다. 실제 회의 세 건에서는 오교정이
//! 0건으로 보였지만, 한국어 녹취록에 영어 단어가 드물어서 가려졌을 뿐이었다.
//! 영어 사전 전체를 넣어보니 임계값에 따라 **50~1300개 단어가 잘못 교정됐다**
//! — `abuse`→`Azure`, `adult`→`Vault`, `large`→`Argo`, `adapt`→`LDAP`.
//! 실측한 오인식 `vmure`(거리 2)가 살아남는 설정은 전부 오교정 400건 이상이었고,
//! 가장 조인 설정조차 20건이었다. 안전한 임계값이 없다.
//!
//! 별칭 방식이 잃는 게 없다는 게 중요하다. whisper의 오인식은 무작위가 아니라
//! 체계적으로 반복된다 — `vmure`는 한 회의에서 16번 나왔다. 한 줄 추가가 오류
//! 16개를 지운다. 새 오인식은 `termfix` 예제로 남은 영문 토큰을 훑어서 찾는다.
//!
//! 한국어 고유명사 교정은 아직 없다. 실측에서 한국어는 전부 정확했고
//! ("전북은행" 11/11) 임계값을 정할 오류 표본이 없었다.

use std::collections::HashMap;

/// 업계 표준 용어의 정답 표기. 대소문자만 맞추는 데 쓴다.
///
/// 같은 용어가 `s3`/`S3`/`S3.`로 갈리면 요약 모델이 다른 것으로 취급한다.
/// 실제 회의 한 건에서 이 정규화만으로 25건이 정리됐다.
const CATALOG: &[&str] = &[
    // 가상화
    "VMware", "vCenter", "vSphere", "ESXi", "vMotion", "vSAN", "OVF", "OVA",
    "Nutanix", "Openstack", "Hyper-V",
    // 컨테이너·오케스트레이션
    "Kubernetes", "Kube", "K8s", "Docker", "Helm", "Istio",
    // 퍼블릭 클라우드
    "AWS", "EC2", "S3", "RDS", "EKS", "ECS", "VPC", "IAM", "EBS", "ELB",
    "Lambda", "CloudFront", "Route53", "Azure", "GCP", "NCP", "NKS",
    // CI/CD·형상관리
    "Jenkins", "ArgoCD", "GitLab", "GitHub", "Git", "npm", "Gradle",
    "Maven", "Nexus", "SonarQube", "Harbor", "CI", "CD", "CICD",
    // 운영·도구
    "Terraform", "Ansible", "Prometheus", "Grafana", "Vault", "Nginx",
    "Redis", "Kafka", "MySQL", "PostgreSQL", "MongoDB", "Elasticsearch",
    "CMP", "CSP", "MSP", "SaaS", "PaaS", "IaaS", "IaC", "DevOps", "SRE",
    // 인프라·네트워크
    "GPU", "CPU", "RAM", "SSD", "NAS", "SAN", "CDN", "DNS", "NAT", "ACL",
    "VPN", "WAF", "LDAP", "SSO", "MFA", "SSL", "TLS", "SSH", "HTTP", "HTTPS",
    // 일반
    "API", "SDK", "CLI", "SLA", "PoC", "SMS", "SNS", "KPI", "QA", "UI", "UX",
];

/// 실제 녹음에서 확인한 오인식 → 정답 표기.
///
/// 영어 단어와 겹치지 않는 것만 넣는다. 겹치면 그 단어를 말했을 때 망가진다.
/// 왼쪽은 대소문자를 구분하지 않고 비교한다.
const ALIASES: &[(&str, &str)] = &[
    // 전북은행 회의(18분)에서 16번 연속으로 이렇게 나왔다.
    ("vmure", "VMware"),
    ("Qube", "Kube"),
];

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

/// 이 토큰을 어떤 정답 표기로 바꿔야 하는지 찾는다. 바꿀 필요가 없으면 `None`.
///
/// 두 단계 모두 **알려진 문자열과 정확히 일치할 때만** 동작한다. 모르는 토큰은
/// 절대 건드리지 않는다 — 이게 오교정이 불가능한 이유다.
fn canonical_for(token: &str, catalog: &[String], lookup: &HashMap<String, usize>) -> Option<String> {
    let lower = token.to_ascii_lowercase();

    // 1단계 — 표기 분열 정리.
    if let Some(&i) = lookup.get(&lower) {
        return (catalog[i] != token).then(|| catalog[i].clone());
    }

    // 2단계 — 확인된 오인식 치환.
    ALIASES
        .iter()
        .find(|(wrong, _)| wrong.eq_ignore_ascii_case(token))
        .map(|(_, right)| (*right).to_string())
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

    #[test]
    fn fixes_aliased_short_form() {
        assert_eq!(correct("Qube 클러스터", ""), "Kube 클러스터");
    }

    /// **가장 중요한 회귀 테스트.**
    ///
    /// 근사 매칭을 쓰던 때 이 단어들이 전부 잘못 교정됐다. 편집거리 매칭을 다시
    /// 들이면 여기서 걸린다. 영어 사전 전체로 재보니 임계값에 따라 50~1300개
    /// 단어가 이런 식으로 망가졌다.
    #[test]
    fn never_touches_ordinary_english_words() {
        for w in ["large", "abuse", "adult", "adapt", "amen", "aging", "arable",
                  "open", "help", "cube", "acute", "adore", "allure"] {
            assert_eq!(correct(w, ""), w, "{w} 를 건드렸다");
        }
    }

    /// 카탈로그에 없고 별칭에도 없는 토큰은 그대로 둔다.
    #[test]
    fn leaves_unknown_tokens_alone() {
        assert_eq!(correct("PMPM 이랑 ROG 얘기", ""), "PMPM 이랑 ROG 얘기");
    }

    /// cicd 회의에서 확인한 표기 분열.
    #[test]
    fn normalizes_cicd_vocabulary() {
        assert_eq!(correct("K8S 에 IAC 로 NPM 설치", ""), "K8s 에 IaC 로 npm 설치");
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
