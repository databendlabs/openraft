(ns jepsen.openraft.scenario.leader-outside-joint-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen.openraft.client :as http]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.openraft.scenario.leader-outside-joint :as scenario]))

(def test-config
  {:nodes ["n1" "n2" "n3"]
   :api-port 21001})

(defn- step [f value]
  {:process :nemesis :type :info :f f :value value})

(def ^:private covered
  [(step :prepare-joint {:status :prepared
                         :committed-voter-configs [#{"n1" "n2" "n3"}
                                                   #{"n1" "n3"}]})
   (step :transfer-to-old-half {:status :transferred :leader "n2"})])

(def ^:private recovered
  (step :check-leader-outside-joint-recovery
        {:status :recovered
         :fresh "after-change"
         :sentinel "before-change"}))

(deftest accepts-a-refused-change-that-recovers
  (is (true? (:valid? (scenario/verdict
                       test-config
                       (conj covered
                             (step :change-from-old-half
                                   {:status :refused
                                    :error-variant :LeaderNotInJointMembership})
                             recovered))))))

(deftest rejects-any-other-answer-to-the-change
  (doseq [change [{:status :changed}
                  {:status :refused :error-variant :InProgress}
                  {:status :refused :error-variant :UncommittedLeaderLog}
                  {:status :not-leader}]]
    (testing (pr-str change)
      (let [result (scenario/verdict
                    test-config
                    (conj covered
                          (step :change-from-old-half change)
                          recovered))]
        (is (false? (:valid? result)))
        (is (false? (:refused? result)))))))

(deftest rejects-an-unanswered-change-or-a-stuck-cluster
  (testing "a change the leader never answers, as when its core panics"
    (let [result (scenario/verdict
                  test-config
                  (conj covered
                        (step :change-from-old-half
                              {:status :indeterminate :kind :transport-error})
                        recovered))]
      (is (false? (:valid? result)))
      (is (true? (:scenario-covered? result)))))

  (testing "a refused change after which the cluster never recovers"
    (is (false? (:valid? (scenario/verdict
                          test-config
                          (conj covered
                                (step :change-from-old-half
                                      {:status :refused
                                       :error-variant :LeaderNotInJointMembership})
                                (step :check-leader-outside-joint-recovery
                                      {:status :timeout})))))))

  (testing "a transfer that never reached the old half is not coverage"
    (let [result (scenario/verdict
                  test-config
                  [(first covered)
                   (step :transfer-to-old-half {:status :transferred
                                                :leader "n1"})
                   (step :change-from-old-half {:status :refused})
                   recovered])]
      (is (false? (:scenario-covered? result))))))

(deftest classifies-the-change-response
  (doseq [[label thrown expected]
          [["a refusal"
            (ex-info "refused" {:kind :openraft-error
                                :error {:ChangeMembershipError
                                        {:LeaderNotInJointMembership {}}}})
            {:status :refused :error-variant :LeaderNotInJointMembership :attempts 1}]
           ["a lost leadership"
            (ex-info "forward" {:kind :openraft-error
                                :error {:ForwardToLeader {:leader_id "n1"}}})
            {:status :not-leader :attempts 1}]
           ["no answer"
            (ex-info "reset" {:kind :transport-error})
            {:status :indeterminate :kind :transport-error :attempts 1}]]]
    (testing label
      (with-redefs [http/change-membership! (fn [endpoint node-ids]
                                              (is (= "n2:21001" endpoint))
                                              (is (= ["n3"] node-ids))
                                              (throw thrown))]
        (is (= expected (#'scenario/change! test-config))))))

  (testing "a refusal that only means X is not ready yet is retried"
    (let [answers (atom [:InProgress :UncommittedLeaderLog
                         :LeaderNotInJointMembership])]
      (with-redefs [http/change-membership!
                    (fn [& _]
                      (let [variant (first @answers)]
                        (swap! answers rest)
                        (throw (ex-info "refused"
                                        {:kind :openraft-error
                                         :error {:ChangeMembershipError
                                                 {variant {}}}}))))]
        (is (= {:status :refused
                :error-variant :LeaderNotInJointMembership
                :attempts 3}
               (#'scenario/change! test-config))))))

  (testing "an unknown error is a Harness failure"
    (with-redefs [http/change-membership! (fn [& _]
                                            (throw (IllegalStateException. "bug")))]
      (is (thrown? IllegalStateException (#'scenario/change! test-config))))))

(deftest recovery-completes-a-bridge-that-dropped-a-voter
  (let [bridge {:leader "n3"
                :stable? false
                :effective-voter-configs [#{"n1" "n3"} #{"n3"}]}
        statuses (atom [bridge bridge (assoc bridge
                                             :stable? true
                                             :voters #{"n3"}
                                             :effective-voter-configs [#{"n3"}])])
        changes (atom [])]
    (with-redefs [cluster/membership-status
                  (fn [_test]
                    (let [status (first @statuses)]
                      (when (next @statuses)
                        (swap! statuses rest))
                      status))
                  http/change-membership!
                  (fn [endpoint node-ids]
                    (swap! changes conj [endpoint node-ids])
                    (when (= ["n1" "n2" "n3"] node-ids)
                      (throw (ex-info "not a learner"
                                      {:kind :openraft-error
                                       :error {:ChangeMembershipError
                                               {:LearnerNotFound {:node_id "n2"}}}}))))
                  http/write! (fn [& _] {:value "after-change" :version 9})
                  http/linearizable-read! (fn [_ k]
                                            {:value (if (= "leader-outside-joint-fresh" k)
                                                      "after-change"
                                                      "before-change")
                                             :version 9})]
      (let [result (#'scenario/recover! test-config)]
        (is (= :recovered (:status result)))
        (is (= #{"n3"} (:voters result)))
        (is (= [["n3:21001" ["n1" "n2" "n3"]]
                ["n3:21001" ["n3"]]]
               (take 2 @changes))
            "n2 is no longer a learner, so the new half {n3} completes the joint config")))))
