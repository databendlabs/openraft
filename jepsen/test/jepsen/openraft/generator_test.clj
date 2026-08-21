(ns jepsen.openraft.generator-test
  (:require [clojure.test :refer [deftest is]]
            [jepsen.generator :as gen]
            [jepsen.generator.test :as gen-test]
            [jepsen.openraft.generator :as openraft-generator]
            [jepsen.openraft.harness :as harness]))

(deftest stops-new-operations-after-a-harness-failure
  (let [failure-state (harness/failure-state)
        generator (openraft-generator/stop-on-harness-failure
                   failure-state
                   [{:f :first} {:f :second}])
        [operation generator'] (gen/op generator
                                       gen-test/default-test
                                       gen-test/default-context)]
    (is (= :first (:f operation)))
    (harness/record-failure! failure-state
                             :client
                             {:operation operation}
                             (RuntimeException. "client failed"))
    (is (nil? (gen/op generator'
                      gen-test/default-test
                      gen-test/default-context)))))

(deftest forwards-in-flight-updates-after-a-harness-failure
  (let [failure-state (harness/failure-state)
        events (atom [])
        delegate (gen/on-update
                  (fn [generator _test _context event]
                    (swap! events conj event)
                    generator)
                  (repeat {:f :ordinary-workload}))
        generator (openraft-generator/stop-on-harness-failure
                   failure-state
                   delegate)
        [operation generator'] (gen/op generator
                                       gen-test/default-test
                                       gen-test/default-context)
        completion (assoc operation :type :ok)]
    (harness/record-failure! failure-state
                             :client
                             {:operation operation}
                             (RuntimeException. "client failed"))
    (let [after-invocation (gen/update generator'
                                       gen-test/default-test
                                       gen-test/default-context
                                       operation)
          after-completion (gen/update after-invocation
                                       gen-test/default-test
                                       gen-test/default-context
                                       completion)]
      (is (= [operation completion] @events))
      (is (nil? (gen/op after-completion
                        gen-test/default-test
                        gen-test/default-context))))))
